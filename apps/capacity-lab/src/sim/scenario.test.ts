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
    expect(result.events.find((event) => event.type === "BLOCK_PUBLISHED")?.time).toBe(0);
  });

  it("keeps Work DA and Block simulation paths separate", () => {
    const da = groupDaSimulation(30, 0.7, BLOCK.d3lSlotMb * 0.7, 42);
    const block = groupBlockSimulation(30, 0.7, BLOCK.mb, 239, 42);
    expect(da.readySeconds).not.toBe(block.readySeconds);
    expect(block.verifySeconds).toBeGreaterThan(0);
    expect(block.packagesPerNode).toBe(Math.ceil(239 / 30));
  });

  it("computes Block → 2/3 Ready from logical slots", () => {
    const result = buildScenario(defaults, 42);
    expect(result.logicalSlots).toHaveLength(1023);
    expect(result.events.filter((event) => event.type === "LOGICAL_READY")).toHaveLength(1023);
    expect(result.metrics.effectiveInterval).toBe(result.metrics.blockTwoThird);
    expect(result.metrics.logicalReadyP90).toBeGreaterThanOrEqual(result.metrics.logicalReadyP50);
    expect(result.metrics.logicalReadyP99).toBeGreaterThanOrEqual(result.metrics.logicalReadyP90);
  });

  it("shows expected 1 → 3 → 10 node improvement with diminishing returns", () => {
    const values = [1, 3, 10, 30, 100, 300].map((nodesPerGroup) => buildScenario({ ...defaults, groupShare: 1, nodesPerGroup }, 42));
    expect(values[1].metrics.groupReadyP50!).toBeLessThan(values[0].metrics.groupReadyP50!);
    expect(values[2].metrics.groupReadyP50!).toBeLessThan(values[1].metrics.groupReadyP50!);
    expect(values[2].metrics.daStoredPerNodeMb).toBeGreaterThan(values[3].metrics.daStoredPerNodeMb);
    expect(values[2].metrics.workPackagesPerNode).toBeGreaterThan(values[3].metrics.workPackagesPerNode);
    expect(values[5].metrics.groupCoordinationSeconds).toBeGreaterThan(values[0].metrics.groupCoordinationSeconds);
  });

  it("changes structure across Group share sensitivity points", () => {
    const shares = [0, 0.25, 0.5, 0.75, 1].map((groupShare) => buildScenario({ ...defaults, groupShare, nodesPerGroup: 10 }, 42));
    expect(shares.map((result) => result.metrics.groupedSlots)).toEqual([0, 256, 512, 767, 1023]);
    expect(shares[0].metrics.groupReadyP50).toBeNull();
    expect(shares[4].metrics.ordinaryReadyP50).toBeNull();
    expect(new Set(shares.map((result) => result.metrics.blockTwoThird)).size).toBeGreaterThan(1);
  });

  it("responds to network load while preserving deterministic outputs", () => {
    const lowLoad = buildScenario({ ...defaults, networkLoad: 0.1 }, 42);
    const highLoad = buildScenario({ ...defaults, networkLoad: 1 }, 42);
    expect(highLoad.metrics.workDaTwoThird).toBeGreaterThanOrEqual(lowLoad.metrics.workDaTwoThird);
    expect(highLoad.metrics.blockTwoThird).toBeGreaterThanOrEqual(lowLoad.metrics.blockTwoThird);
    expect(buildScenario(defaults, 42)).toEqual(buildScenario(defaults, 42));
  });

  it("handles extreme inputs without invalid metrics", () => {
    const cases = [
      { groupShare: 0, nodesPerGroup: 1, ethBlocksPerItem: 0.25, networkLoad: 0.01 },
      { groupShare: 1, nodesPerGroup: 300, ethBlocksPerItem: 500, networkLoad: 1 },
    ];
    const [noGroup, allGroup] = cases.map((params) => buildScenario(params, 42));
    expect(noGroup.metrics.groupReadyP50).toBeNull();
    expect(noGroup.metrics.homeWanUtilization).toBe(0);
    expect(allGroup.metrics.ordinaryReadyP50).toBeNull();
    expect(allGroup.metrics.groupedSlots).toBe(1023);
    for (const result of [noGroup, allGroup]) {
      expect(result.metrics.physicalNodes).toBeGreaterThanOrEqual(1023);
      expect(result.metrics.effectiveInterval).toBeGreaterThan(0);
      expect(result.logicalSlots.every((slot) => slot.daReady >= 0 && slot.blockReady >= 0)).toBe(true);
      for (const value of Object.values(result.metrics)) if (typeof value === "number") expect(Number.isFinite(value)).toBe(true);
    }
  });
});
