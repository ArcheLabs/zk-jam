import { describe, expect, it } from "vitest";
import { BLOCK } from "../model/constants";
import { globalNetworkRttSeconds } from "../model/formulas";
import { REGIONS, buildScenario } from "./scenario";
import { groupBlockSimulation, groupDaSimulation } from "./group";
import { simulateBlockRound } from "./network";

const defaults = { groupShare: 0.3, nodesPerGroup: 30, ethBlocksPerItem: 1, networkLoad: 0.7 };

describe("Geographically Local Honest Group Model v2", () => {
  it("uses proof as a synchronous barrier and shifts Block to report time", () => {
    const result = buildScenario(defaults, 42);
    expect(result.metrics.workDaTwoThird).toBeGreaterThan(0);
    expect(result.metrics.proofBarrierSeconds).toBeCloseTo(3.9, 8);
    expect(result.metrics.reportReadySeconds).toBeCloseTo(4.24, 8);
    expect(result.metrics.roundTimeSeconds).toBeCloseTo(result.metrics.reportReadySeconds + result.metrics.blockToTwoThirdSeconds, 9);
    expect(result.events.find((event) => event.type === "BLOCK_PUBLISHED")?.time).toBe(result.metrics.reportReadySeconds);
    expect(result.events.find((event) => event.type === "QUORUM_2_3")?.time).toBe(result.metrics.roundTimeSeconds);
    const logical = result.events.find((event) => event.type === "LOGICAL_READY" && event.slot === 0);
    expect(logical?.time).toBe(result.metrics.reportReadySeconds + result.logicalSlots[0].blockReady);
  });

  it("keeps Work DA and Block simulation paths separate", () => {
    const da = groupDaSimulation(30, 0.7, BLOCK.d3lSlotMb * 0.7, 42);
    const block = groupBlockSimulation(30, 0.7, BLOCK.mb, 239, 42);
    expect(da.readySeconds).not.toBe(block.readySeconds);
    expect(block.verifySeconds).toBeGreaterThan(0);
    expect(block.packagesPerNode).toBe(Math.ceil(239 / 30));
  });

  it("keeps 20–30ms local RTT and sub-300ms global RTT in seconds", () => {
    const result = buildScenario(defaults, 42);
    expect(result.metrics.groupIngressFanoutSeconds).toBeGreaterThanOrEqual(0.02);
    expect(result.metrics.groupIngressFanoutSeconds).toBeLessThanOrEqual(0.09);
    expect(globalNetworkRttSeconds(REGIONS[0], REGIONS[4])).toBeLessThan(0.3);
  });

  it("adds physical Group traffic to shared resources", () => {
    const result = buildScenario({ ...defaults, groupShare: 1 }, 42);
    const enabled = simulateBlockRound(BLOCK.mb, 30, 0.7, 1023, REGIONS, result.runtime, true);
    const disabled = simulateBlockRound(BLOCK.mb, 30, 0.7, 1023, REGIONS, result.runtime, false);
    expect(result.metrics.physicalProtocolEndpoints).toBe(30 * 1023);
    expect(result.metrics.groupAdditionalTrafficMb).toBeGreaterThan(0);
    expect(enabled.backboneDemandMb).toBeGreaterThan(disabled.backboneDemandMb);
    expect(enabled.sourceDemandMb).toBeGreaterThan(disabled.sourceDemandMb);
  });

  it("changes physical network structure across Group share points", () => {
    const shares = [0, 0.25, 0.5, 0.75, 1].map((groupShare) => buildScenario({ ...defaults, groupShare, nodesPerGroup: 30 }, 42));
    expect(shares.map((result) => result.metrics.physicalProtocolEndpoints)).toEqual([1023, 8447, 15871, 23266, 30690]);
    expect(shares[0].metrics.groupAdditionalTrafficMb).toBe(0);
    for (let index = 1; index < shares.length; index += 1) {
      expect(shares[index].metrics.groupAdditionalTrafficMb).toBeGreaterThan(shares[index - 1].metrics.groupAdditionalTrafficMb);
      expect(shares[index].metrics.backboneDemandMb).toBeGreaterThan(shares[index - 1].metrics.backboneDemandMb);
    }
    expect(shares[0].metrics.groupReadyP50).toBeNull();
    expect(shares[4].metrics.ordinaryReadyP50).toBeNull();
  });

  it("shows node benefits and coordination costs without forcing monotonic Block latency", () => {
    const values = [1, 3, 10, 30, 100, 300].map((nodesPerGroup) => buildScenario({ ...defaults, groupShare: 1, nodesPerGroup }, 42));
    for (let index = 1; index < values.length; index += 1) {
      expect(values[index].metrics.physicalProtocolEndpoints).toBeGreaterThan(values[index - 1].metrics.physicalProtocolEndpoints);
      expect(values[index].metrics.groupAdditionalTrafficMb).toBeGreaterThan(values[index - 1].metrics.groupAdditionalTrafficMb);
      expect(values[index].metrics.groupIngressFanoutSeconds).toBeGreaterThan(values[index - 1].metrics.groupIngressFanoutSeconds);
    }
    expect(values[1].metrics.groupReadyP50!).toBeLessThan(values[0].metrics.groupReadyP50!);
    expect(values[2].metrics.groupReadyP50!).toBeLessThan(values[1].metrics.groupReadyP50!);
    expect(values[1].metrics.daStoredPerNodeMb).toBeLessThan(values[0].metrics.daStoredPerNodeMb);
    expect(values[4].metrics.daStoredPerNodeMb).toBeLessThan(values[3].metrics.daStoredPerNodeMb);
    expect(values[1].metrics.workPackagesPerNode).toBeLessThan(values[0].metrics.workPackagesPerNode);
    expect(values[4].metrics.workPackagesPerNode).toBeLessThan(values[3].metrics.workPackagesPerNode);
  });

  it("keeps proof independent from Group share and network propagation", () => {
    const shares = [0, 0.25, 0.5, 0.75, 1].map((groupShare) => buildScenario({ ...defaults, groupShare }, 42));
    expect(new Set(shares.map((result) => result.metrics.proofBarrierSeconds)).size).toBe(1);
    const proofCases = [0.25, 1, 2, 5].map((ethBlocksPerItem) => buildScenario({ ...defaults, ethBlocksPerItem }, 42));
    for (let index = 1; index < proofCases.length; index += 1) {
      expect(proofCases[index].metrics.proofBarrierSeconds).toBeGreaterThan(proofCases[index - 1].metrics.proofBarrierSeconds);
      expect(proofCases[index].metrics.reportReadySeconds).toBeGreaterThan(proofCases[index - 1].metrics.reportReadySeconds);
      expect(proofCases[index].metrics.roundTimeSeconds).toBeGreaterThan(proofCases[index - 1].metrics.roundTimeSeconds);
      expect(proofCases[index].metrics.blockToTwoThirdSeconds).toBeCloseTo(proofCases[0].metrics.blockToTwoThirdSeconds, 9);
    }
  });

  it("handles extreme inputs and remains deterministic", () => {
    const cases = [
      { groupShare: 0, nodesPerGroup: 1, ethBlocksPerItem: 0.25, networkLoad: 0.01 },
      { groupShare: 1, nodesPerGroup: 300, ethBlocksPerItem: 500, networkLoad: 1 },
    ];
    const [noGroup, allGroup] = cases.map((params) => buildScenario(params, 42));
    expect(noGroup.metrics.groupReadyP50).toBeNull();
    expect(allGroup.metrics.ordinaryReadyP50).toBeNull();
    expect(buildScenario(defaults, 42)).toEqual(buildScenario(defaults, 42));
    for (const result of [noGroup, allGroup]) {
      expect(result.metrics.roundTimeSeconds).toBeGreaterThan(0);
      expect(result.logicalSlots).toHaveLength(1023);
      expect(result.logicalSlots.every((slot) => slot.daReady >= 0 && slot.blockReady >= 0)).toBe(true);
      for (const value of Object.values(result.metrics)) if (typeof value === "number") expect(Number.isFinite(value)).toBe(true);
    }
  });
});
