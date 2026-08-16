import { activeDataNodes, daShardPerNodeMb, effectiveHomeDownMbps, groupInternalSeconds, workerQueueSeconds } from "../model/formulas";
import { HOME_NODE } from "../model/constants";

export type GroupDaResult = {
  activeDataNodes: number;
  shardMb: number;
  externalFetchSeconds: number;
  replicationSeconds: number;
  coordinationSeconds: number;
  controlTrafficSeconds: number;
  readySeconds: number;
  daMbpsPerNode: number;
};

export type GroupBlockResult = GroupDaResult & {
  verifySeconds: number;
  packagesPerNode: number;
};

function externalFetch(payloadMb: number, nodes: number, load: number) {
  const activeNodes = activeDataNodes(nodes, payloadMb);
  const shardMb = daShardPerNodeMb(payloadMb, nodes);
  return { activeNodes, shardMb, seconds: 0.05 + shardMb / Math.max(0.001, effectiveHomeDownMbps(load) / 8) };
}

export function groupDaSimulation(nodes: number, load: number, payloadMb: number, seed = 42): GroupDaResult {
  const fetch = externalFetch(payloadMb, nodes, load);
  const internal = groupInternalSeconds(payloadMb, nodes, load, seed);
  return {
    activeDataNodes: fetch.activeNodes,
    shardMb: fetch.shardMb,
    externalFetchSeconds: fetch.seconds,
    replicationSeconds: internal.replicationSeconds,
    coordinationSeconds: internal.coordinationSeconds,
    controlTrafficSeconds: internal.controlTrafficSeconds,
    readySeconds: fetch.seconds + internal.totalSeconds,
    daMbpsPerNode: (fetch.shardMb / Math.max(0.001, fetch.seconds)) * 8,
  };
}

export function groupBlockSimulation(nodes: number, load: number, payloadMb: number, verifyTasks: number, seed = 42): GroupBlockResult {
  const da = groupDaSimulation(nodes, load, payloadMb, seed);
  const packagesPerNode = Math.ceil(verifyTasks / Math.max(1, Math.min(Math.max(1, Math.round(nodes)), Math.max(1, verifyTasks))));
  const verifySeconds = workerQueueSeconds(packagesPerNode, HOME_NODE.verifyWorkers, 0.025);
  return { ...da, verifySeconds, packagesPerNode };
}
