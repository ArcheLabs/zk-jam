# ZkJAM Capacity Lab

The static browser application lives in `apps/capacity-lab`. It is a Vite + React + TypeScript site designed for GitHub Pages at `/zk-jam/`. The world map is a locally bundled Natural Earth 1:110m Admin 0 Countries GeoJSON file rendered by deck.gl; it has no tiles, map API, CDN runtime, or API key.

## Model

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

Work DA includes external fetch, replication, coordination, and control traffic. Block propagation includes global propagation and block data replication; Group logical readiness then combines header/data readiness, Group verification, and final coordination. The main metric is Block → 2/3 Ready, calculated directly from block publication at `t = 0` using the logical-ready quantile. There is no six-second slot floor and no proof/refine offset added to Block timing.

Global region RTT is clamped to 20–250 ms and is always handled as seconds internally. Quorum remains 1,023 logical weights; a Group changes physical node count and resource sharing, not consensus weight.

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
