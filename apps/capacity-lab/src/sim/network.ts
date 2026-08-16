import { GLOBAL_NETWORK, PROTOCOL } from "../model/constants";
import { activeDataNodes, effectiveHomeDownMbps, effectiveHomeUpMbps, globalNetworkRttSeconds, groupFinalizationSeconds, groupIngressFanoutSeconds, groupInternalSeconds, groupTrafficComponents, networkLoadFactor, ordinaryIngressMbps } from "../model/formulas";
import type { Region, Runtime } from "../model/types";
import { runFluidFlows, type Flow, type FlowCompletion, type Resource } from "./engine";

export type RoundNetworkResult = {
  readyTimes: number[];
  globalArrivalTimes: number[];
  groupDataTimes: number[];
  groupedReadyTimes: number[];
  ordinaryReadyTimes: number[];
  groupedExternalFetchTimes: number[];
  ordinaryExternalFetchTimes: number[];
  sourceUtilization: number;
  backboneUtilization: number;
  homeWanUtilization: number;
  groupAdditionalTrafficMb: number;
  physicalProtocolEndpoints: number;
  groupIngressFanoutSeconds: number;
  groupFinalizationSeconds: number;
  groupCommunicationSeconds: number;
  sourceDemandMb: number;
  backboneDemandMb: number;
};

export const groupedSlot = (index: number, grouped: number) => ((index * 2654435761) >>> 0) % PROTOCOL.logicalValidators < grouped;

const jitter = (index: number) => 0.86 + (((index * 1664525 + 1013904223) >>> 0) / 4_294_967_296) * 0.28;

type GroupBackground = { internalId: string; memberId: string; region: Region };
type FlowContext = {
  flows: Flow[];
  primaryFlows: Flow[];
  grouped: boolean[];
  regions: Region[];
  source: Resource;
  backbone: Resource[];
  homeWan: Resource[];
  backgrounds: Array<GroupBackground | null>;
  groupAdditionalTrafficMb: number;
  physicalProtocolEndpoints: number;
};

/**
 * Group primary flows still carry one unique external payload. Additional
 * replication/control/member traffic is represented as real shared flows.
 */
function buildFlows(payloadMb: number, nodes: number, load: number, groupedSlots: number, regions: Region[], sourceCapacityMbps: number, includeGroupBackground: boolean): FlowContext {
  const efficiency = networkLoadFactor(load);
  const globalSource: Resource = { id: `global-source-${sourceCapacityMbps}`, capacityMbps: sourceCapacityMbps * efficiency };
  const backbone = regions.map((region) => ({ id: `backbone-${region.name}`, capacityMbps: GLOBAL_NETWORK.regionalBackboneMbps * efficiency }));
  const homeWan: Resource[] = [];
  const primaryFlows: Flow[] = [];
  const backgroundFlows: Flow[] = [];
  const grouped: boolean[] = [];
  const backgrounds: Array<GroupBackground | null> = [];
  let groupIndex = 0;
  let groupAdditionalTrafficMb = 0;
  for (let index = 0; index < PROTOCOL.logicalValidators; index += 1) {
    const isGroup = groupedSlot(index, groupedSlots);
    const region = regions[index % regions.length];
    const groupDown = isGroup ? { id: `group-home-down-${groupIndex}`, capacityMbps: activeDataNodes(nodes, payloadMb) * effectiveHomeDownMbps(load) } : null;
    const groupUp = isGroup ? { id: `group-home-up-${groupIndex}`, capacityMbps: activeDataNodes(nodes, payloadMb) * effectiveHomeUpMbps(load) } : null;
    const terminal = groupDown ?? { id: `ordinary-wan-${index}`, capacityMbps: ordinaryIngressMbps(load) };
    if (isGroup && groupDown) homeWan.push(groupDown);
    const primary = { id: `flow-${index}`, remainingMB: payloadMb, resources: [globalSource, backbone[index % backbone.length], terminal] };
    primaryFlows.push(primary);
    grouped.push(isGroup);
    if (isGroup && groupDown && groupUp) {
      const components = groupTrafficComponents(payloadMb, nodes);
      const internalId = `group-internal-${groupIndex}`;
      const memberId = `group-member-${groupIndex}`;
      if (includeGroupBackground) {
        backgroundFlows.push({ id: internalId, remainingMB: components.replicationTrafficMb + components.internalControlMb, resources: [backbone[index % backbone.length], groupUp, groupDown] });
        backgroundFlows.push({ id: memberId, remainingMB: components.physicalMemberTrafficMb, resources: [globalSource, backbone[index % backbone.length], groupDown] });
      }
      backgrounds.push({ internalId, memberId, region });
      groupAdditionalTrafficMb += components.groupAdditionalTrafficMb;
      groupIndex += 1;
    } else {
      backgrounds.push(null);
    }
  }
  return {
    flows: [...primaryFlows, ...backgroundFlows], primaryFlows, grouped, regions, source: globalSource, backbone, homeWan, backgrounds,
    groupAdditionalTrafficMb, physicalProtocolEndpoints: PROTOCOL.logicalValidators - groupedSlots + groupedSlots * Math.max(1, Math.round(nodes)),
  };
}

function completionMap(completions: FlowCompletion[]) {
  return new Map(completions.map((completion) => [completion.id, completion.time]));
}

function utilization(flows: Flow[], resource: Resource, makespan: number) {
  if (!flows.length || makespan <= 0) return 0;
  const demandMb = flows.filter((flow) => flow.resources.some((candidate) => candidate.id === resource.id)).reduce((sum, flow) => sum + flow.remainingMB, 0);
  return Math.min(1, demandMb / ((resource.capacityMbps / 8) * makespan));
}

function demand(flows: Flow[], resource: Resource) {
  return flows.filter((flow) => flow.resources.some((candidate) => candidate.id === resource.id)).reduce((sum, flow) => sum + flow.remainingMB, 0);
}

function summarize(context: FlowContext, makespan: number) {
  return {
    sourceUtilization: utilization(context.flows, context.source, makespan),
    backboneUtilization: Math.max(0, ...context.backbone.map((resource) => utilization(context.flows, resource, makespan))),
    homeWanUtilization: Math.max(0, ...context.homeWan.map((resource) => utilization(context.flows, resource, makespan))),
    sourceDemandMb: demand(context.flows, context.source),
    backboneDemandMb: Math.max(0, ...context.backbone.map((resource) => demand(context.flows, resource))),
  };
}

function runRound(payloadMb: number, nodes: number, load: number, groupedSlots: number, regions: Region[], runtime: Runtime, sourceCapacityMbps: number, includeGroupBackground: boolean): RoundNetworkResult {
  const context = buildFlows(payloadMb, nodes, load, groupedSlots, regions, sourceCapacityMbps, includeGroupBackground);
  const completions = runFluidFlows(context.flows);
  const times = completionMap(completions);
  const primaryMakespan = Math.max(0, ...context.primaryFlows.map((flow) => times.get(flow.id) ?? 0));
  const ingress = groupIngressFanoutSeconds(nodes, runtime.seed);
  const finalization = groupFinalizationSeconds(nodes, runtime.seed);
  const globalArrivalTimes = context.primaryFlows.map((flow, index) => {
    const region = context.regions[index % context.regions.length];
    const target = runtime.producer;
    return (times.get(flow.id) ?? 0) + globalNetworkRttSeconds(region, target) * jitter(index);
  });
  const groupDataTimes = context.primaryFlows.map((flow, index) => {
    const background = context.backgrounds[index];
    const backgroundReady = background && includeGroupBackground ? Math.max(times.get(background.internalId) ?? 0, times.get(background.memberId) ?? 0) : 0;
    return context.grouped[index] ? Math.max(globalArrivalTimes[index], backgroundReady) : globalArrivalTimes[index];
  });
  const readyTimes = groupDataTimes.map((value, index) => context.grouped[index] ? value + ingress : value);
  const groupedReadyTimes = readyTimes.filter((_, index) => context.grouped[index]);
  const ordinaryReadyTimes = readyTimes.filter((_, index) => !context.grouped[index]);
  const groupedExternalFetchTimes = globalArrivalTimes.filter((_, index) => context.grouped[index]);
  const ordinaryExternalFetchTimes = globalArrivalTimes.filter((_, index) => !context.grouped[index]);
  const groupInternal = groupInternalSeconds(payloadMb, nodes, load, runtime.seed);
  return {
    readyTimes, globalArrivalTimes, groupDataTimes, groupedReadyTimes, ordinaryReadyTimes, groupedExternalFetchTimes, ordinaryExternalFetchTimes,
    ...summarize(context, primaryMakespan),
    groupAdditionalTrafficMb: context.groupAdditionalTrafficMb,
    physicalProtocolEndpoints: context.physicalProtocolEndpoints,
    groupIngressFanoutSeconds: ingress,
    groupFinalizationSeconds: finalization,
    groupCommunicationSeconds: groupInternal.replicationSeconds + groupInternal.controlTrafficSeconds + ingress + finalization,
  };
}

export const simulateWorkDaRound = (payloadMb: number, nodes: number, load: number, groupedSlots: number, regions: Region[], runtime: Runtime, includeGroupBackground = true) =>
  runRound(payloadMb, nodes, load, groupedSlots, regions, runtime, GLOBAL_NETWORK.daSourceEgressMbps, includeGroupBackground);

export const simulateBlockRound = (payloadMb: number, nodes: number, load: number, groupedSlots: number, regions: Region[], runtime: Runtime, includeGroupBackground = true) =>
  runRound(payloadMb, nodes, load, groupedSlots, regions, runtime, GLOBAL_NETWORK.blockSourceEgressMbps, includeGroupBackground);
