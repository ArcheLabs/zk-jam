import { describe, expect, it } from "vitest";
import { BLOCK } from "../model/constants";
import { buildScenario } from "./scenario";
import { groupBlockSimulation, groupDaSimulation } from "./group";

const defaults = { groupShare: 0.3, nodesPerGroup: 30, ethBlocksPerItem: 1, networkLoad: 0.7 };

describe("capacity scenario sanity", () => {
  it("keeps DA latency in seconds after ms conversion", () => {
    const result = buildScenario(defaults, 42);
    expect(result.metrics.workDaTwoThird).toBeGreaterThan(0);
    expect(result.metrics.workDaTwoThird).toBeLessThan(2);
  });

  it("uses separate DA and block group paths", () => {
    const da = groupDaSimulation(30, 0.7, BLOCK.d3lSlotMb * 0.7);
    const block = groupBlockSimulation(30, 0.7, BLOCK.mb, 239);
    expect(da.readySeconds).not.toBe(block.dataSeconds);
    expect(block.verifySeconds).toBeGreaterThan(0);
  });

  it("computes quorum over logical slots and emits a complete event stream", () => {
    const result = buildScenario(defaults, 42);
    expect(result.logicalSlots).toHaveLength(1023);
    expect(result.events.filter((event) => event.type === "LOGICAL_READY")).toHaveLength(1023);
    expect(result.metrics.effectiveInterval).toBeCloseTo(result.metrics.proofGate + result.metrics.blockTwoThird, 8);
  });
});
