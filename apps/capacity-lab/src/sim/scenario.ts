import { BLOCK, GROUP_NETWORK, HOME_NODE, ORDINARY_VALIDATOR, PROTOCOL, ZK_REFERENCE } from "../model/constants";
import { clamp, daShardMb, gasFromEthBlocks, groupInternalSeconds, proofSeconds, quantile, workerQueueSeconds } from "../model/formulas";
import type { LogicalSlotResult, Parameters, Region, Runtime, SimulationEvent, SimulationResult } from "../model/types";
import { groupBlockSimulation, groupDaSimulation } from "./group";
import { groupedSlot, simulateBlockRound, simulateWorkDaRound } from "./network";

export const REGIONS: Region[] = [
  { name: "Tokyo", longitude: 139.7, latitude: 35.7 }, { name: "Singapore", longitude: 103.8, latitude: 1.3 },
  { name: "Sydney", longitude: 151.2, latitude: -33.9 }, { name: "Frankfurt", longitude: 8.7, latitude: 50.1 },
  { name: "Virginia", longitude: -77.4, latitude: 37.5 }, { name: "São Paulo", longitude: -46.6, latitude: -23.5 },
  { name: "California", longitude: -122.4, latitude: 37.8 }, { name: "Mumbai", longitude: 72.9, latitude: 19.1 },
  { name: "London", longitude: -0.1, latitude: 51.5 }, { name: "Johannesburg", longitude: 28.0, latitude: -26.2 },
  { name: "Toronto", longitude: -79.4, latitude: 43.7 }, { name: "Seoul", longitude: 126.9, latitude: 37.5 },
];

const seeded = (seed: number) => { let value = seed >>> 0; return () => { value ^= value << 13; value ^= value >>> 17; value ^= value << 5; return (value >>> 0) / 4_294_967_296; }; };
function runtime(seed: number): Runtime {
  const random = seeded(seed);
  const shuffled = [...REGIONS].sort(() => random() - 0.5);
  return { seed, producer: shuffled[0], provers: shuffled.slice(1, 4), sources: shuffled.slice(4, 8) };
}

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
  const groupDa = groupedSlots ? groupDaSimulation(nodes, load, shard, seed) : null;
  const groupBlock = groupedSlots ? groupBlockSimulation(nodes, load, BLOCK.mb, activeCores, seed) : null;
  const workNetwork = simulateWorkDaRound(shard, nodes, load, groupedSlots, REGIONS, rt);
  const blockNetwork = simulateBlockRound(BLOCK.mb, nodes, load, groupedSlots, REGIONS, rt);
  const proofAvg = proofSeconds(eth);
  const proofP99 = eth * ZK_REFERENCE.openVmEthBlockP99Seconds;
  const itemsPerPackage = Math.max(1, Math.min(PROTOCOL.maxWorkItemsPerPackage, Math.floor(150 / eth)));
  const refine = PROTOCOL.slotSeconds * (eth / 150);
  const workDaTwoThird = quantile(workNetwork.readyTimes, 2 / 3);
  const workReport = refine + Math.max(proofAvg, workDaTwoThird) + ZK_REFERENCE.aggregateSeconds;
  const ordinaryVerify = workerQueueSeconds(activeCores, ORDINARY_VALIDATOR.verifyWorkers, 0.025);
  const groupVerify = groupBlock?.verifySeconds ?? 0;
  const groupFinalCoordination = groupBlock?.coordinationSeconds ?? 0;
  const logicalSlots: LogicalSlotResult[] = Array.from({ length: PROTOCOL.logicalValidators }, (_, index) => {
    const region = REGIONS[index % REGIONS.length];
    const grouped = groupedSlot(index, groupedSlots);
    const data = blockNetwork.readyTimes[index];
    const verify = grouped ? groupVerify + (groupBlock?.controlTrafficSeconds ?? 0) : ordinaryVerify;
    const finalCoordination = grouped ? groupFinalCoordination : 0;
    const blockReady = Math.max(data, verify, 0.005) + finalCoordination + 0.01;
    return { index, grouped, region, daReady: workNetwork.readyTimes[index], blockReady };
  });
  const readyTimes = logicalSlots.map((slot) => slot.blockReady);
  const blockTwoThird = quantile(readyTimes, 2 / 3);
  const logicalReadyP50 = quantile(readyTimes, 0.50);
  const logicalReadyP90 = quantile(readyTimes, 0.90);
  const logicalReadyP99 = quantile(readyTimes, 0.99);
  const groupDaP50 = workNetwork.groupedReadyTimes.length ? quantile(workNetwork.groupedReadyTimes, 0.5) : null;
  const ordinaryDaP50 = workNetwork.ordinaryReadyTimes.length ? quantile(workNetwork.ordinaryReadyTimes, 0.5) : null;
  const groupReadyP50 = groupedSlots ? quantile(logicalSlots.filter((slot) => slot.grouped).map((slot) => slot.blockReady), 0.5) : null;
  const ordinaryReadyP50 = ordinarySlots ? quantile(logicalSlots.filter((slot) => !slot.grouped).map((slot) => slot.blockReady), 0.5) : null;
  const effectiveInterval = blockTwoThird;
  const physicalNodes = ordinarySlots + groupedSlots * nodes;
  const groupInternal = groupDa ? groupInternalSeconds(shard, nodes, load, seed) : { replicationSeconds: 0, coordinationSeconds: 0, controlTrafficSeconds: 0, totalSeconds: 0 };
  const groupPackagesPerNode = groupBlock?.packagesPerNode ?? 0;
  const groupCriticalValues = { DATA: groupDa ? quantile(blockNetwork.groupedReadyTimes, 0.5) : 0, VERIFY: groupVerify, "INTERNAL NETWORK": groupInternal.coordinationSeconds + groupInternal.controlTrafficSeconds };
  const groupCriticalPath = (Object.entries(groupCriticalValues).sort(([, left], [, right]) => right - left)[0]?.[0] ?? "DATA") as SimulationResult["metrics"]["groupCriticalPath"];
  const sourceUtilization = Math.max(workNetwork.sourceUtilization, blockNetwork.sourceUtilization);
  const backboneUtilization = Math.max(workNetwork.backboneUtilization, blockNetwork.backboneUtilization);
  const homeWanUtilization = Math.max(workNetwork.homeWanUtilization, blockNetwork.homeWanUtilization);
  const totalWorkers = ordinarySlots * ORDINARY_VALIDATOR.verifyWorkers + groupedSlots * nodes * HOME_NODE.verifyWorkers;
  const verifyDemandSeconds = PROTOCOL.logicalValidators * activeCores * 0.025;
  const verifyUtilization = clamp(verifyDemandSeconds / (Math.max(1, totalWorkers) * Math.max(0.001, blockTwoThird)), 0, 1);
  const bottleneckScores = { DATA: Math.max(sourceUtilization, backboneUtilization, homeWanUtilization), VERIFY: verifyUtilization, "INTERNAL NETWORK": groupCriticalValues["INTERNAL NETWORK"] / Math.max(0.001, groupReadyP50 ?? blockTwoThird), SOURCE: sourceUtilization, BACKBONE: backboneUtilization, SLOT: 0 };
  const dominantBottleneck = (Object.entries(bottleneckScores).sort(([, left], [, right]) => right - left)[0]?.[0] ?? "DATA") as SimulationResult["metrics"]["dominantBottleneck"];
  const requiredClusters = activeCores * Math.min(150, eth * itemsPerPackage) * ZK_REFERENCE.openVmEthBlockAvgSeconds / PROTOCOL.slotSeconds;
  const blockDataReady = quantile(blockNetwork.readyTimes, 0.5);
  const blockVerifyReady = Math.max(ordinaryVerify, groupVerify + (groupBlock?.controlTrafficSeconds ?? 0));
  const events: SimulationEvent[] = [
    { time: 0, type: "WORK_START" }, { time: refine, type: "REFINE_DONE" },
    { time: refine, type: "PROOF_START" }, { time: refine, type: "DA_START" },
    { time: refine + proofAvg, type: "PROOF_DONE" }, { time: refine + workDaTwoThird, type: "DA_2_3_READY" },
    { time: workReport, type: "REPORT_READY" }, { time: 0, type: "BLOCK_PUBLISHED", region: rt.producer },
    { time: 0.01, type: "BLOCK_HEADER_RECEIVED", region: rt.producer },
    { time: blockDataReady, type: "BLOCK_DATA_READY", region: rt.producer },
    { time: blockVerifyReady, type: "BLOCK_VERIFY_READY", region: rt.producer },
    ...logicalSlots.map((slot) => ({ time: slot.blockReady, type: "LOGICAL_READY" as const, slot: slot.index, region: slot.region })),
    { time: blockTwoThird, type: "QUORUM_2_3" },
  ].sort((a, b) => a.time - b.time) as SimulationEvent[];
  return {
    params: { groupShare, nodesPerGroup: nodes, ethBlocksPerItem: eth, networkLoad: load },
    runtime: rt,
    metrics: {
      physicalNodes, groupedSlots, ordinarySlots, activeCores, gasEquivalent: gasFromEthBlocks(eth), itemsPerPackage,
      proofAvg, proofP99, workDaTwoThird, workReport, blockTwoThird, effectiveInterval,
      logicalReadyP50, logicalReadyP90, logicalReadyP99, groupDaP50, ordinaryDaP50, groupReadyP50, ordinaryReadyP50,
      smallNodeDaMbps: groupDa?.daMbpsPerNode ?? 0,
      daStoredPerNodeMb: groupDa ? (shard * GROUP_NETWORK.replicationFactor) / Math.max(1, groupDa.activeDataNodes) : 0,
      workPackagesPerNode: groupPackagesPerNode,
      groupExternalFetchSeconds: workNetwork.groupedExternalFetchTimes.length ? quantile(workNetwork.groupedExternalFetchTimes, 0.5) : null,
      groupInternalReplicationSeconds: groupInternal.replicationSeconds,
      groupCoordinationSeconds: groupInternal.coordinationSeconds,
      groupControlTrafficSeconds: groupInternal.controlTrafficSeconds,
      groupVerifySeconds: groupVerify,
      groupCriticalPath, requiredClusters, sourceUtilization, backboneUtilization, homeWanUtilization, verifyUtilization,
      dominantBottleneck, pressure: proofAvg > effectiveInterval ? "PROVER-BOUND" : "NETWORK / VERIFY",
    }, events, logicalSlots,
  };
}
