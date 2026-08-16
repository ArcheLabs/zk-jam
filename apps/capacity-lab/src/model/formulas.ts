import { BLOCK, GROUP_NETWORK, HOME_NODE, ORDINARY_VALIDATOR, PROTOCOL, ZK_REFERENCE } from "./constants";
import type { Region } from "./types";

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

export const networkLoadFactor = (load: number) => clamp(1 - 0.45 * clamp(load, 0, 1), 0.45, 1);
export const effectiveHomeDownMbps = (load: number) => HOME_NODE.wanDownMbps * HOME_NODE.networkEfficiency * networkLoadFactor(load);
export const effectiveHomeUpMbps = (load: number) => HOME_NODE.wanUpMbps * HOME_NODE.networkEfficiency * networkLoadFactor(load);

export const activeDataNodes = (nodes: number, totalDaMb: number) => Math.min(Math.max(1, Math.round(nodes)), Math.max(1, Math.ceil(totalDaMb / GROUP_NETWORK.minDaShardMb)));
export const daShardPerNodeMb = (totalDaMb: number, nodes: number) => totalDaMb / activeDataNodes(nodes, totalDaMb);
export const groupRttSeconds = (seed = 42) => GROUP_NETWORK.minRttSeconds + (((seed * 1664525 + 1013904223) >>> 0) / 4_294_967_296) * (GROUP_NETWORK.maxRttSeconds - GROUP_NETWORK.minRttSeconds);
export const globalNetworkRttSeconds = (a: Region, b: Region) => {
  const normalizedDistance = Math.hypot((a.longitude - b.longitude) / 360, (a.latitude - b.latitude) / 180);
  return clamp((20 + normalizedDistance * 205) / 1000, 0.020, 0.250);
};
export const coordinationRounds = (nodes: number) => nodes <= 1 ? 0 : Math.ceil(Math.log(Math.max(1, nodes)) / Math.log(GROUP_NETWORK.overlayFanout));
export const groupCoordinationSeconds = (nodes: number, seed = 42) => coordinationRounds(nodes) * groupRttSeconds(seed);

export const groupInternalSeconds = (payloadMb: number, nodes: number, load: number, seed = 42) => {
  const activeNodes = activeDataNodes(nodes, payloadMb);
  const totalReplicationMb = payloadMb * (GROUP_NETWORK.replicationFactor - 1);
  const replicationUploadPerNodeMb = totalReplicationMb / activeNodes;
  const uploadSeconds = replicationUploadPerNodeMb / Math.max(0.001, effectiveHomeUpMbps(load) / 8);
  const downloadSeconds = replicationUploadPerNodeMb / Math.max(0.001, effectiveHomeDownMbps(load) / 8);
  const replicationSeconds = Math.max(uploadSeconds, downloadSeconds);
  const rounds = coordinationRounds(nodes);
  const controlMb = (Math.max(1, Math.round(nodes)) * rounds * GROUP_NETWORK.controlBytesPerNode) / 1_000_000;
  const controlTrafficSeconds = rounds === 0 ? 0 : Math.max(controlMb / Math.max(0.001, effectiveHomeUpMbps(load) / 8), controlMb / Math.max(0.001, effectiveHomeDownMbps(load) / 8));
  const coordinationSeconds = groupCoordinationSeconds(nodes, seed);
  return { replicationSeconds, coordinationSeconds, controlTrafficSeconds, totalSeconds: replicationSeconds + coordinationSeconds + controlTrafficSeconds };
};

export const workerQueueSeconds = (taskCount: number, workers: number, taskSeconds: number) =>
  Math.ceil(taskCount / Math.max(1, Math.min(taskCount, workers))) * taskSeconds;

export const ordinaryIngressMbps = (load: number) => ORDINARY_VALIDATOR.wanMbps * 0.85 * networkLoadFactor(load);
export const daShardMb = (load = 1) => BLOCK.d3lSlotMb * clamp(load, 0, 1);
