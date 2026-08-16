import { BLOCK, ORDINARY_VALIDATOR, PROTOCOL, SMALL_VALIDATOR, ZK_REFERENCE } from "./constants";

export const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));
export const quantile = (values: number[], p: number) => {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * p))];
};

export const gasFromEthBlocks = (ethBlocks: number) => (ethBlocks / 150) * PROTOCOL.refineGasPerPackage;
export const formatGas = (gas: number) => gas >= 1e9 ? `${(gas / 1e9).toFixed(2)}B` : `${(gas / 1e6).toFixed(1)}M`;
export const proofSeconds = (ethBlocks: number) => ethBlocks * ZK_REFERENCE.openVmEthBlockAvgSeconds;
export const p99ProofSeconds = (ethBlocks: number) => ethBlocks * ZK_REFERENCE.openVmEthBlockP99Seconds;

export const sharedGroupIngressMbps = (nodes: number, load: number) => {
  const independent = nodes * SMALL_VALIDATOR.wanDownMbps * 0.85;
  const contention = 0.95 - 0.31 * load;
  return Math.min(independent, 2_000) * Math.max(0.48, contention);
};

export const localReplicationSeconds = (payloadMb: number, nodes: number, replication = SMALL_VALIDATOR.replication) => {
  const copies = Math.min(replication, Math.max(1, nodes)) - 1;
  if (copies <= 0 || payloadMb <= 0) return 0;
  const totalMb = copies * payloadMb;
  const perNodeMb = totalMb / Math.max(1, nodes);
  const portMbPerSecond = (SMALL_VALIDATOR.lanPortMbps / 8) * 0.8;
  const fabricMbPerSecond = (SMALL_VALIDATOR.groupFabricMbps / 8) * 0.8;
  const portBound = perNodeMb / portMbPerSecond;
  const fabricBound = totalMb / fabricMbPerSecond;
  const coordination = (SMALL_VALIDATOR.localRttMs / 1000) * Math.ceil(Math.log2(Math.max(1, nodes)));
  return Math.max(portBound, fabricBound) + coordination;
};

export const workerQueueSeconds = (taskCount: number, workers: number, taskSeconds: number) =>
  Math.ceil(taskCount / Math.max(1, Math.min(taskCount, workers))) * taskSeconds;

export const ordinaryIngressMbps = (load: number) => ORDINARY_VALIDATOR.wanMbps * 0.85 * Math.max(0.48, 0.95 - 0.31 * load);
export const daShardMb = (load = 1) => BLOCK.d3lSlotMb * clamp(load, 0, 1);
