import { localReplicationSeconds, sharedGroupIngressMbps, workerQueueSeconds } from "../model/formulas";
import { SMALL_VALIDATOR } from "../model/constants";
import { runFluidFlows, type Resource } from "./engine";

export type GroupMicroResult = { externalSeconds: number; replicationSeconds: number; verifySeconds: number; readySeconds: number; daMbpsPerNode: number };

export function simulateRepresentativeGroup(nodes: number, load: number, payloadMb: number, verifyTasks: number): GroupMicroResult {
  const count = Math.max(1, nodes);
  const nodeLinks: Resource[] = Array.from({ length: count }, (_, index) => ({ id: `wan-${index}`, capacityMbps: SMALL_VALIDATOR.wanDownMbps }));
  const fabric: Resource = { id: "group-ingress", capacityMbps: 2_000 };
  const sliceMb = payloadMb / count;
  const flows = Array.from({ length: count }, (_, index) => ({ id: `slice-${index}`, remainingMB: sliceMb, resources: [nodeLinks[index], fabric] }));
  const externalSeconds = runFluidFlows(flows).reduce((max, result) => Math.max(max, result.time), 0) / (0.85 * Math.max(0.48, 0.95 - 0.31 * load));
  const replicationSeconds = localReplicationSeconds(payloadMb, count);
  const verifyWorkers = Math.min(verifyTasks, count * SMALL_VALIDATOR.verifyWorkersPerNode);
  const verifySeconds = workerQueueSeconds(verifyTasks, verifyWorkers, 0.025);
  return {
    externalSeconds,
    replicationSeconds,
    verifySeconds,
    readySeconds: externalSeconds + replicationSeconds,
    daMbpsPerNode: (sliceMb / Math.max(0.001, externalSeconds)) * 8,
  };
}

export const groupIngressForDisplay = (nodes: number, load: number) => sharedGroupIngressMbps(nodes, load);
