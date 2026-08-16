export type SourceTag = "PROTOCOL" | "JAM MODEL" | "MEASURED" | "OPENVM REFERENCE" | "DERIVED" | "ZKJAM DESIGN";

export const PROTOCOL = {
  slotSeconds: 6,
  cores: 341,
  logicalValidators: 1023,
  refineGasPerPackage: 5_000_000_000,
  maxWorkItemsPerPackage: 16,
  source: "PROTOCOL" as SourceTag,
};

export const ORDINARY_VALIDATOR = {
  wanMbps: 500,
  verifyWorkers: 8,
  source: "JAM MODEL" as SourceTag,
};

export const HOME_NODE = {
  wanDownMbps: 100,
  wanUpMbps: 20,
  cpuCores: 4,
  verifyWorkers: 2,
  networkEfficiency: 0.75,
  source: "ZKJAM DESIGN" as SourceTag,
};

export const ZK_REFERENCE = {
  openVmEthBlockAvgSeconds: 3.9,
  openVmEthBlockP99Seconds: 6.3,
  aggregateProofBytes: 2048,
  aggregateSeconds: 0.30,
  source: "OPENVM REFERENCE" as SourceTag,
};

export const BLOCK = { mb: 16, d3lSlotMb: 12.76704, source: "JAM MODEL" as SourceTag };

export const GLOBAL_NETWORK = {
  daSourceEgressMbps: 100_000,
  blockSourceEgressMbps: 120_000,
  regionalBackboneMbps: 30_000,
  source: "ZKJAM DESIGN" as SourceTag,
};

export const GROUP_NETWORK = {
  minDaShardMb: 0.25,
  replicationFactor: 3,
  overlayFanout: 4,
  controlBytesPerNode: 8 * 1024,
  minRttSeconds: 0.020,
  maxRttSeconds: 0.030,
  source: "ZKJAM DESIGN" as SourceTag,
};
