import type { SourceTag } from "./constants";

export type Parameters = {
  groupShare: number;
  nodesPerGroup: number;
  ethBlocksPerItem: number;
  networkLoad: number;
};

export type Region = { name: string; longitude: number; latitude: number };
export type Runtime = { seed: number; producer: Region; provers: Region[]; sources: Region[] };
export type Metric = { value: string; source: SourceTag };
export type LogicalSlotResult = { index: number; grouped: boolean; region: Region; daReady: number; blockReady: number };
export type Bottleneck = "DATA" | "VERIFY" | "INTERNAL NETWORK" | "SOURCE" | "BACKBONE" | "SLOT";
export type SimulationEvent = {
  time: number;
  type: "WORK_START" | "REFINE_DONE" | "PROOF_START" | "DA_START" | "PROOF_DONE" | "DA_2_3_READY" | "REPORT_READY" | "BLOCK_PUBLISHED" | "BLOCK_HEADER_RECEIVED" | "BLOCK_DATA_READY" | "BLOCK_VERIFY_READY" | "LOGICAL_READY" | "QUORUM_2_3";
  slot?: number;
  region?: Region;
};

export type SimulationMetrics = {
  physicalNodes: number;
  groupedSlots: number;
  ordinarySlots: number;
  activeCores: number;
  gasEquivalent: number;
  itemsPerPackage: number;
  proofAvg: number;
  proofP99: number;
  workDaTwoThird: number;
  workReport: number;
  blockTwoThird: number;
  effectiveInterval: number;
  logicalReadyP50: number;
  logicalReadyP90: number;
  logicalReadyP99: number;
  groupDaP50: number | null;
  ordinaryDaP50: number | null;
  groupReadyP50: number | null;
  ordinaryReadyP50: number | null;
  smallNodeDaMbps: number;
  daStoredPerNodeMb: number;
  workPackagesPerNode: number;
  groupExternalFetchSeconds: number | null;
  groupInternalReplicationSeconds: number;
  groupCoordinationSeconds: number;
  groupControlTrafficSeconds: number;
  groupVerifySeconds: number;
  groupCriticalPath: "DATA" | "VERIFY" | "INTERNAL NETWORK";
  requiredClusters: number;
  sourceUtilization: number;
  backboneUtilization: number;
  homeWanUtilization: number;
  verifyUtilization: number;
  dominantBottleneck: Bottleneck;
  pressure: "PROVER-BOUND" | "NETWORK / VERIFY";
};

export type SimulationResult = {
  params: Parameters;
  runtime: Runtime;
  metrics: SimulationMetrics;
  events: SimulationEvent[];
  logicalSlots: LogicalSlotResult[];
};
