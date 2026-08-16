import { describe, expect, it } from "vitest";
import { daShardMb, localReplicationSeconds } from "./formulas";

describe("capacity formulas", () => {
  it("scales the D3L shard with network load", () => {
    expect(daShardMb(1)).toBeCloseTo(12.76704, 5);
    expect(daShardMb(0.5)).toBeCloseTo(6.38352, 5);
  });

  it("keeps replication traffic explicit and non-zero for a 3x group", () => {
    expect(localReplicationSeconds(16, 30, 3)).toBeGreaterThan(0);
    expect(localReplicationSeconds(16, 30, 3)).toBeLessThan(localReplicationSeconds(16, 2, 3));
  });
});
