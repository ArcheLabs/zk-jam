import { describe, expect, it } from "vitest";
import { BLOCK, GROUP_NETWORK, HOME_NODE } from "./constants";
import { activeDataNodes, daShardMb, effectiveHomeDownMbps, effectiveHomeUpMbps, globalNetworkRttSeconds, groupInternalSeconds, groupRttSeconds, networkLoadFactor } from "./formulas";

describe("capacity formulas", () => {
  it("scales the D3L shard with network load", () => {
    expect(daShardMb(1)).toBeCloseTo(12.76704, 5);
    expect(daShardMb(0.5)).toBeCloseTo(6.38352, 5);
  });

  it("uses Mbps / 8 for home-WAN byte transfer and explicit 3x replication", () => {
    expect(HOME_NODE.wanDownMbps / 8).toBe(12.5);
    expect(effectiveHomeDownMbps(0) / 8).toBeCloseTo(9.375, 6);
    expect(effectiveHomeUpMbps(0) / 8).toBeCloseTo(1.875, 6);
    const internal = groupInternalSeconds(16, 30, 0, 42);
    const expectedUpload = (16 * (GROUP_NETWORK.replicationFactor - 1) / activeDataNodes(30, 16)) / (effectiveHomeUpMbps(0) / 8);
    expect(internal.replicationSeconds).toBeCloseTo(expectedUpload, 6);
    expect(internal.replicationSeconds).toBeGreaterThan(0);
  });

  it("keeps group RTT in milliseconds and global RTT bounded", () => {
    expect(groupRttSeconds(42)).toBeGreaterThanOrEqual(GROUP_NETWORK.minRttSeconds);
    expect(groupRttSeconds(42)).toBeLessThanOrEqual(GROUP_NETWORK.maxRttSeconds);
    const tokyo = { name: "Tokyo", longitude: 139.7, latitude: 35.7 };
    const virginia = { name: "Virginia", longitude: -77.4, latitude: 37.5 };
    expect(globalNetworkRttSeconds(tokyo, virginia)).toBeGreaterThanOrEqual(0.020);
    expect(globalNetworkRttSeconds(tokyo, virginia)).toBeLessThan(0.3);
  });

  it("activates only the data nodes needed for the minimum shard", () => {
    expect(activeDataNodes(300, BLOCK.d3lSlotMb)).toBe(Math.ceil(BLOCK.d3lSlotMb / GROUP_NETWORK.minDaShardMb));
    expect(activeDataNodes(3, 16)).toBe(3);
    expect(networkLoadFactor(0)).toBe(1);
    expect(networkLoadFactor(1)).toBe(0.55);
  });
});
