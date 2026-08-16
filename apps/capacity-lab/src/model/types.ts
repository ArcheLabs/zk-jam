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
export type SimulationEvent = {
  time: number;
  type: "WORK_START" | "REFINE_DONE" | "PROOF_START" | "DA_START" | "PROOF_DONE" | "DA_2_3_READY" | "REPORT_READY" | "BLOCK_PUBLISHED" | "BLOCK_HEADER_RECEIVED" | "LOGICAL_READY" | "QUORUM_2_3";
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
  proofGate: number;
  blockTwoThird: number;
  effectiveInterval: number;
  groupDaP50: number | null;
  ordinaryDaP50: number | null;
  groupReadyP50: number | null;
  ordinaryReadyP50: number | null;
  smallNodeDaMbps: number;
  requiredClusters: number;
  pressure: "PROVER-BOUND" | "NETWORK / VERIFY";
};

export type SimulationResult = {
  params: Parameters;
  runtime: Runtime;
  metrics: SimulationMetrics;
  events: SimulationEvent[];
  logicalSlots: LogicalSlotResult[];
};
