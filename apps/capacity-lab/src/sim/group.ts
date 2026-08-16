import { localReplicationSeconds, sharedGroupIngressMbps, workerQueueSeconds } from "../model/formulas";
import { SMALL_VALIDATOR } from "../model/constants";
import { runFluidFlows, type Resource } from "./engine";

type GroupFetchResult = { externalSeconds: number; replicationSeconds: number; readySeconds: number; mbpsPerNode: number };
export type GroupDaResult = GroupFetchResult;
export type GroupBlockResult = GroupFetchResult & { verifySeconds: number; dataSeconds: number };

function simulateGroupFetch(nodes: number, load: number, payloadMb: number): GroupFetchResult {
  const count = Math.max(1, Math.round(nodes));
  const nodeLinks: Resource[] = Array.from({ length: count }, (_, index) => ({ id: `wan-${index}`, capacityMbps: SMALL_VALIDATOR.wanDownMbps }));
  const fabric: Resource = { id: "group-ingress", capacityMbps: 2_000 };
  const sliceMb = payloadMb / count;
  const flows = Array.from({ length: count }, (_, index) => ({ id: `slice-${index}`, remainingMB: sliceMb, resources: [nodeLinks[index], fabric] }));
  const rawExternalSeconds = runFluidFlows(flows).reduce((max, result) => Math.max(max, result.time), 0);
  const efficiency = 0.85 * Math.max(0.48, 0.95 - 0.31 * load);
  const externalSeconds = rawExternalSeconds / efficiency;
  const replicationSeconds = localReplicationSeconds(payloadMb, count);
  return {
    externalSeconds,
    replicationSeconds,
    readySeconds: externalSeconds + replicationSeconds,
    mbpsPerNode: (sliceMb / Math.max(0.001, externalSeconds)) * 8,
  };
}

/** DA path: cooperative shard fetch followed by explicit local replication. All values are seconds. */
export function groupDaSimulation(nodes: number, load: number, payloadMb: number): GroupDaResult {
  return simulateGroupFetch(nodes, load, payloadMb);
}

/** Block path: independent block fetch/replication plus a separate verify-worker queue. */
export function groupBlockSimulation(nodes: number, load: number, payloadMb: number, verifyTasks: number): GroupBlockResult {
  const fetch = simulateGroupFetch(nodes, load, payloadMb);
  const verifyWorkers = Math.min(verifyTasks, Math.max(1, Math.round(nodes)) * SMALL_VALIDATOR.verifyWorkersPerNode);
  const verifySeconds = workerQueueSeconds(verifyTasks, verifyWorkers, 0.025);
  return { ...fetch, dataSeconds: fetch.readySeconds, verifySeconds };
}

export const groupIngressForDisplay = (nodes: number, load: number) => sharedGroupIngressMbps(nodes, load);
