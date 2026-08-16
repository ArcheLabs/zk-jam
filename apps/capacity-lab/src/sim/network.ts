import { GLOBAL_NETWORK, PROTOCOL } from "../model/constants";
import { activeDataNodes, effectiveHomeDownMbps, groupInternalSeconds, globalNetworkRttSeconds, networkLoadFactor, ordinaryIngressMbps } from "../model/formulas";
import type { Region, Runtime } from "../model/types";
import { runFluidFlows, type Flow, type FlowCompletion, type Resource } from "./engine";

export type RoundNetworkResult = {
  readyTimes: number[];
  groupedReadyTimes: number[];
  ordinaryReadyTimes: number[];
  groupedExternalFetchTimes: number[];
  ordinaryExternalFetchTimes: number[];
  sourceUtilization: number;
  backboneUtilization: number;
  homeWanUtilization: number;
};

export const groupedSlot = (index: number, grouped: number) => ((index * 2654435761) >>> 0) % PROTOCOL.logicalValidators < grouped;

const jitter = (index: number) => 0.86 + (((index * 1664525 + 1013904223) >>> 0) / 4_294_967_296) * 0.28;

type FlowContext = {
  flows: Flow[];
  grouped: boolean[];
  regions: Region[];
  source: Resource;
  backbone: Resource[];
  homeWan: Resource[];
};

/**
 * Group flows use one analytical resource per logical Group. That resource is
 * the aggregate of independent home-node downlinks, not a shared gateway.
 */
function buildFlows(payloadMb: number, nodes: number, load: number, groupedSlots: number, regions: Region[], sourceCapacityMbps: number): FlowContext {
  const efficiency = networkLoadFactor(load);
  const globalSource: Resource = { id: `global-source-${sourceCapacityMbps}`, capacityMbps: sourceCapacityMbps * efficiency };
  const backbone = regions.map((region) => ({ id: `backbone-${region.name}`, capacityMbps: GLOBAL_NETWORK.regionalBackboneMbps * efficiency }));
  const homeWan: Resource[] = [];
  const flows: Flow[] = [];
  const grouped: boolean[] = [];
  let groupIndex = 0;
  for (let index = 0; index < PROTOCOL.logicalValidators; index += 1) {
    const isGroup = groupedSlot(index, groupedSlots);
    const region = regions[index % regions.length];
    const terminal = isGroup
      ? { id: `group-home-down-${groupIndex}`, capacityMbps: activeDataNodes(nodes, payloadMb) * effectiveHomeDownMbps(load) }
      : { id: `ordinary-wan-${index}`, capacityMbps: ordinaryIngressMbps(load) };
    if (isGroup) homeWan.push(terminal);
    const resources: Resource[] = [globalSource, backbone[index % backbone.length], terminal];
    flows.push({ id: `flow-${index}`, remainingMB: payloadMb, resources });
    grouped.push(isGroup);
    if (isGroup) groupIndex += 1;
  }
  return { flows, grouped, regions, source: globalSource, backbone, homeWan };
}

function completionMap(completions: FlowCompletion[]) {
  return new Map(completions.map((completion) => [completion.id, completion.time]));
}

function utilization(flows: Flow[], resource: Resource, makespan: number) {
  if (!flows.length || makespan <= 0) return 0;
  const demandMb = flows.filter((flow) => flow.resources.some((candidate) => candidate.id === resource.id)).reduce((sum, flow) => sum + flow.remainingMB, 0);
  return Math.min(1, demandMb / ((resource.capacityMbps / 8) * makespan));
}

function summarize(context: FlowContext, makespan: number) {
  return {
    sourceUtilization: utilization(context.flows, context.source, makespan),
    backboneUtilization: Math.max(0, ...context.backbone.map((resource) => utilization(context.flows, resource, makespan))),
    homeWanUtilization: Math.max(0, ...context.homeWan.map((resource) => utilization(context.flows, resource, makespan))),
  };
}

function runRound(payloadMb: number, nodes: number, load: number, groupedSlots: number, regions: Region[], runtime: Runtime, sourceCapacityMbps: number, includeFullInternal: boolean): RoundNetworkResult {
  const context = buildFlows(payloadMb, nodes, load, groupedSlots, regions, sourceCapacityMbps);
  const completions = runFluidFlows(context.flows);
  const times = completionMap(completions);
  const makespan = Math.max(0, ...completions.map((completion) => completion.time));
  const readyTimes = context.flows.map((flow, index) => {
    const region = context.regions[index % context.regions.length];
    const target = includeFullInternal ? runtime.sources[index % runtime.sources.length] : runtime.producer;
    const internal = context.grouped[index]
      ? (includeFullInternal ? groupInternalSeconds(payloadMb, nodes, load, runtime.seed).totalSeconds : groupInternalSeconds(payloadMb, nodes, load, runtime.seed).replicationSeconds)
      : 0;
    return (times.get(flow.id) ?? 0) + globalNetworkRttSeconds(region, target) * jitter(index) + internal;
  });
  const groupedReadyTimes = readyTimes.filter((_, index) => context.grouped[index]);
  const ordinaryReadyTimes = readyTimes.filter((_, index) => !context.grouped[index]);
  const groupedExternalFetchTimes = context.flows.filter((_, index) => context.grouped[index]).map((flow, index) => {
    const logicalIndex = context.flows.findIndex((candidate) => candidate.id === flow.id);
    const region = context.regions[logicalIndex % context.regions.length];
    const target = includeFullInternal ? runtime.sources[logicalIndex % runtime.sources.length] : runtime.producer;
    return (times.get(flow.id) ?? 0) + globalNetworkRttSeconds(region, target) * jitter(logicalIndex);
  });
  const ordinaryExternalFetchTimes = context.flows.filter((_, index) => !context.grouped[index]).map((flow, index) => {
    const logicalIndex = context.flows.findIndex((candidate) => candidate.id === flow.id);
    const region = context.regions[logicalIndex % context.regions.length];
    const target = includeFullInternal ? runtime.sources[logicalIndex % runtime.sources.length] : runtime.producer;
    return (times.get(flow.id) ?? 0) + globalNetworkRttSeconds(region, target) * jitter(logicalIndex);
  });
  return { readyTimes, groupedReadyTimes, ordinaryReadyTimes, groupedExternalFetchTimes, ordinaryExternalFetchTimes, ...summarize(context, makespan) };
}

export const simulateWorkDaRound = (payloadMb: number, nodes: number, load: number, groupedSlots: number, regions: Region[], runtime: Runtime) =>
  runRound(payloadMb, nodes, load, groupedSlots, regions, runtime, GLOBAL_NETWORK.daSourceEgressMbps, true);

export const simulateBlockRound = (payloadMb: number, nodes: number, load: number, groupedSlots: number, regions: Region[], runtime: Runtime) =>
  runRound(payloadMb, nodes, load, groupedSlots, regions, runtime, GLOBAL_NETWORK.blockSourceEgressMbps, false);
