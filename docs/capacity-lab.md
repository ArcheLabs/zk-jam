# ZkJAM Capacity Lab

The static browser application lives in `apps/capacity-lab`. It is a Vite + React + TypeScript site designed for GitHub Pages at `/zk-jam/`. The world map is a locally bundled Natural Earth 1:110m Admin 0 Countries GeoJSON file rendered by deck.gl; it has no tiles, map API, CDN runtime, or API key.

## Model: Geographically Local Honest Group Model v2

The simulator is a deterministic, round-level fluid discrete-event model. It represents 1,023 logical validator weights, a representative Group micro-simulation, and the global source/backbone resources. The browser event stream is the single timing source for the timeline, playhead, deck.gl ArcLayer/TripsLayer animations, and node readiness.

### Home Group

Each Group is a geographically local cluster of independent residential links, not a datacenter LAN:

- home WAN: 100 Mbps down / 20 Mbps up;
- 4 CPU cores and 2 verification workers per node;
- network efficiency: 0.75;
- Group overlay RTT: deterministic 20–30 ms;
- replication factor: 3;
- coordination fanout: 4, with `ceil(log(nodes) / log(4))` rounds;
- 8 KB control traffic per node per coordination round.

The effective home bandwidth is:

```text
loadFactor = clamp(1 - 0.45 × networkLoad, 0.45, 1)
effectiveDown = 100 × 0.75 × loadFactor Mbps
effectiveUp   = 20  × 0.75 × loadFactor Mbps
```

All byte transfer calculations convert Mbps to MB/s with `Mbps / 8`. Group internal replication uses home upload/download capacity, not a LAN or shared fabric. For payload `P`, the replication traffic is `P × (3 - 1)` total bytes distributed across active data nodes.

### DA and Block paths

Work DA and Block propagation are separate simulations. The D3L shard is `12.76704 × networkLoad` MB, with a minimum DA shard of 0.25 MB. Active data nodes are:

```text
min(nodes, ceil(totalDaMb / 0.25))
```

Work DA and Block propagation are separate simulations. Block primary flows still fetch one unique `BLOCK.mb` payload per logical validator; Group nodes do not each download a full block. A Group's extra replication, control, and physical-member protocol traffic is added as background flows competing for the shared global source, regional backbone, and Group aggregate WAN resources.

The final synchronous timing is:

```text
proofBarrierSeconds = proofSeconds(ethBlocksPerItem)
reportReadySeconds = refine + max(proofBarrierSeconds, workDaTwoThird) + aggregation
blockToTwoThirdSeconds = quantile(logicalSlots.map(slot => slot.blockReady), 2 / 3)
roundTimeSeconds = reportReadySeconds + blockToTwoThirdSeconds
```

`BLOCK_PUBLISHED` occurs at `reportReadySeconds`; every `LOGICAL_READY` event is report time plus its relative Block propagation time; `QUORUM_2_3` occurs at `roundTimeSeconds`. There is no six-second slot floor and no proof/refine offset added to the pure Block → 2/3 metric.

Group network tax is quantified as:

```text
groupAdditionalTrafficMb = replicationTrafficMb
                         + internalControlMb
                         + physicalMemberTrafficMb
```

Replication and control background flows use regional backbone plus aggregate home upload/download. Physical-member flows use the shared protocol/source resource, regional backbone, and aggregate home download. `physicalProtocolEndpoints` equals ordinary logical validators plus `Group slots × nodesPerGroup`, so Group share changes real shared-network demand while preserving one logical consensus weight per Group.

The representative Group communication metric is:

```text
groupCommunicationSeconds = ingress fanout
                           + replication
                           + control traffic
                           + finalization
```

The right rail intentionally keeps only nine decision metrics: Proof / item, Work → Report, Block → 2/3, Logical Ready P99, Group Ready, Ordinary Ready, DA / Group Node, Work / Group Node, and Group Communication. Source, backbone, home-WAN, and verification utilization remain in `SimulationResult` for bottleneck detection and tests. Bottleneck classification uses `PROOF`, `WORK_DA`, `BLOCK_NETWORK`, `VERIFY`, and `GROUP_COMMUNICATION`.

Global region RTT is clamped to 20–250 ms and is always handled as seconds internally. Quorum remains 1,023 logical weights; a Group changes physical node count and resource sharing, not consensus weight. Proof timing remains the OpenVM reference barrier and is independent of Group share and node count.

## Limitations

This is intentionally a capacity instrument rather than a packet-level network emulator. The fluid scheduler models shared source, regional backbone, ordinary validator WAN, and aggregate independent Group home downlinks. It does not model packet loss, retransmission, NAT behavior, ISP peering, disk I/O, a SimGrid runtime, or backend state. Region coordinates and global capacity constants are design assumptions, while OpenVM proof timings are reference inputs. Results should be compared as deterministic sensitivity scenarios, not treated as production capacity guarantees.

## Development

```bash
cd apps/capacity-lab
npm install
npm test
npm run build
npm run dev
```
