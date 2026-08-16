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

export const SMALL_VALIDATOR = {
  wanDownMbps: 100,
  wanUpMbps: 50,
  lanPortMbps: 1_000,
  groupFabricMbps: 10_000,
  verifyWorkersPerNode: 2,
  replication: 3,
  localRttMs: 0.5,
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
