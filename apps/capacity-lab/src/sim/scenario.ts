import { BLOCK, ORDINARY_VALIDATOR, PROTOCOL, SMALL_VALIDATOR, ZK_REFERENCE } from "../model/constants";
import { clamp, daShardMb, gasFromEthBlocks, ordinaryIngressMbps, proofSeconds, quantile, workerQueueSeconds } from "../model/formulas";
import type { LogicalSlotResult, Parameters, Region, Runtime, SimulationEvent, SimulationResult } from "../model/types";
import { groupBlockSimulation, groupDaSimulation } from "./group";

export const REGIONS: Region[] = [
  { name: "Tokyo", longitude: 139.7, latitude: 35.7 }, { name: "Singapore", longitude: 103.8, latitude: 1.3 },
  { name: "Sydney", longitude: 151.2, latitude: -33.9 }, { name: "Frankfurt", longitude: 8.7, latitude: 50.1 },
  { name: "Virginia", longitude: -77.4, latitude: 37.5 }, { name: "São Paulo", longitude: -46.6, latitude: -23.5 },
  { name: "California", longitude: -122.4, latitude: 37.8 }, { name: "Mumbai", longitude: 72.9, latitude: 19.1 },
  { name: "London", longitude: -0.1, latitude: 51.5 }, { name: "Johannesburg", longitude: 28.0, latitude: -26.2 },
  { name: "Toronto", longitude: -79.4, latitude: 43.7 }, { name: "Seoul", longitude: 126.9, latitude: 37.5 },
];

const seeded = (seed: number) => { let value = seed >>> 0; return () => { value ^= value << 13; value ^= value >>> 17; value ^= value << 5; return (value >>> 0) / 4_294_967_296; }; };
const distance = (a: Region, b: Region) => Math.hypot((a.longitude - b.longitude) / 360, (a.latitude - b.latitude) / 180);
const daLatencySeconds = (a: Region, b: Region, factor: number) => ((5 + distance(a, b) * 205) * factor) / 1000;
const blockHeaderSeconds = (a: Region, b: Region, factor: number) => ((7 + distance(a, b) * 225) * factor) / 1000;

function runtime(seed: number): Runtime {
  const random = seeded(seed);
  const shuffled = [...REGIONS].sort(() => random() - 0.5);
  return { seed, producer: shuffled[0], provers: shuffled.slice(1, 4), sources: shuffled.slice(4, 8) };
}

function groupedSlot(index: number, grouped: number) { return ((index * 2654435761) >>> 0) % PROTOCOL.logicalValidators < grouped; }
const jitter = (index: number) => 0.86 + (((index * 1664525 + 1013904223) >>> 0) / 4_294_967_296) * 0.28;

export function buildScenario(params: Parameters, seed = 42): SimulationResult {
  const groupShare = clamp(params.groupShare, 0, 1);
  const nodes = Math.max(1, Math.round(params.nodesPerGroup));
  const eth = clamp(params.ethBlocksPerItem, 0.25, 500);
  const load = clamp(params.networkLoad, 0.01, 1);
  const groupedSlots = Math.round(PROTOCOL.logicalValidators * groupShare);
  const ordinarySlots = PROTOCOL.logicalValidators - groupedSlots;
  const activeCores = Math.max(1, Math.round(PROTOCOL.cores * load));
  const rt = runtime(seed);
  const shard = daShardMb(load);
  const groupDa = groupDaSimulation(nodes, load, shard);
  const groupBlock = groupBlockSimulation(nodes, load, BLOCK.mb, activeCores);
  const ordinaryMbps = ordinaryIngressMbps(load);
  const ordinaryDa = shard / (ordinaryMbps / 8);
  const proofAvg = proofSeconds(eth);
  const proofP99 = eth * ZK_REFERENCE.openVmEthBlockP99Seconds;
  const itemsPerPackage = Math.max(1, Math.min(PROTOCOL.maxWorkItemsPerPackage, Math.floor(150 / eth)));
  const refine = PROTOCOL.slotSeconds * (eth / 150);
  const workDaTimes = Array.from({ length: PROTOCOL.logicalValidators }, (_, index) => {
    const source = rt.sources[index % rt.sources.length];
    const region = REGIONS[index % REGIONS.length];
    return daLatencySeconds(region, source, jitter(index)) + (groupedSlot(index, groupedSlots) && nodes > 1 ? groupDa.readySeconds : ordinaryDa);
  });
  const workDaTwoThird = quantile(workDaTimes, 2 / 3);
  const workReport = refine + Math.max(proofAvg, workDaTwoThird) + ZK_REFERENCE.aggregateSeconds;
  const proofGate = Math.max(PROTOCOL.slotSeconds, workReport);
  const groupVerify = groupBlock.verifySeconds;
  const ordinaryVerify = workerQueueSeconds(activeCores, ORDINARY_VALIDATOR.verifyWorkers, 0.025);
  const logicalSlots: LogicalSlotResult[] = Array.from({ length: PROTOCOL.logicalValidators }, (_, index) => {
    const region = REGIONS[index % REGIONS.length];
    const grouped = groupedSlot(index, groupedSlots) && nodes > 1;
    const header = blockHeaderSeconds(region, rt.producer, jitter(index));
    const data = grouped ? groupBlock.dataSeconds : BLOCK.mb / (ordinaryMbps / 8);
    const verify = grouped ? groupVerify : ordinaryVerify;
    const daCheck = header * 0.35 + 0.005;
    const blockReady = header + Math.max(data, verify, daCheck) + header + 0.01;
    return { index, grouped, region, daReady: workDaTimes[index], blockReady };
  });
  const blockTwoThird = quantile(logicalSlots.map((slot) => slot.blockReady), 2 / 3);
  const groupDaP50 = groupedSlots ? quantile(logicalSlots.filter((slot) => slot.grouped).map((slot) => slot.daReady), 0.5) : null;
  const ordinaryDaP50 = ordinarySlots ? quantile(logicalSlots.filter((slot) => !slot.grouped).map((slot) => slot.daReady), 0.5) : null;
  const groupReadyP50 = groupedSlots ? quantile(logicalSlots.filter((slot) => slot.grouped).map((slot) => slot.blockReady), 0.5) : null;
  const ordinaryReadyP50 = ordinarySlots ? quantile(logicalSlots.filter((slot) => !slot.grouped).map((slot) => slot.blockReady), 0.5) : null;
  const effectiveInterval = proofGate + blockTwoThird;
  const physicalNodes = ordinarySlots + groupedSlots * nodes;
  const requiredClusters = activeCores * Math.min(150, eth * itemsPerPackage) * ZK_REFERENCE.openVmEthBlockAvgSeconds / PROTOCOL.slotSeconds;
  const events: SimulationEvent[] = [
    { time: 0, type: "WORK_START" }, { time: refine, type: "REFINE_DONE" },
    { time: refine, type: "PROOF_START" }, { time: refine, type: "DA_START" },
    { time: refine + proofAvg, type: "PROOF_DONE" }, { time: refine + workDaTwoThird, type: "DA_2_3_READY" },
    { time: workReport, type: "REPORT_READY" }, { time: proofGate, type: "BLOCK_PUBLISHED", region: rt.producer },
    ...logicalSlots.map((slot) => ({ time: proofGate + slot.blockReady, type: "LOGICAL_READY" as const, slot: slot.index, region: slot.region })),
    { time: effectiveInterval, type: "QUORUM_2_3" },
  ].sort((a, b) => a.time - b.time) as SimulationEvent[];
  return {
    params: { groupShare, nodesPerGroup: nodes, ethBlocksPerItem: eth, networkLoad: load },
    runtime: rt,
    metrics: {
      physicalNodes, groupedSlots, ordinarySlots, activeCores, gasEquivalent: gasFromEthBlocks(eth), itemsPerPackage,
      proofAvg, proofP99, workDaTwoThird, workReport, proofGate, blockTwoThird, effectiveInterval,
      groupDaP50, ordinaryDaP50, groupReadyP50, ordinaryReadyP50,
      smallNodeDaMbps: (shard / Math.max(0.001, groupDa.externalSeconds) / Math.max(1, nodes)) * 8,
      requiredClusters, pressure: requiredClusters > 1_000 ? "PROVER-BOUND" : "NETWORK / VERIFY",
    }, events, logicalSlots,
  };
}
