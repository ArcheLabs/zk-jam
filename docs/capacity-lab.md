# ZkJAM Capacity Lab

The static browser application lives in `apps/capacity-lab`. It is a Vite + React + TypeScript site designed for GitHub Pages. The Pages workflow builds the app from that directory and deploys its `dist` output.

The simulator uses a representative Group micro-simulation and 1,023 logical validator results. Its fluid resource engine models constrained WAN links, a shared Group ingress resource, local replication, and verification worker queues without packet-level networking or physical-node object creation. The returned event stream is the only timing source consumed by the timeline and deck.gl map playhead.

Run locally:

```bash
cd apps/capacity-lab
npm install
npm run dev
```
