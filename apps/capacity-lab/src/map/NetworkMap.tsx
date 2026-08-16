import { useEffect, useMemo, useState } from "react";
import DeckGL from "@deck.gl/react";
import { ArcLayer, GeoJsonLayer, ScatterplotLayer } from "@deck.gl/layers";
import { TripsLayer } from "@deck.gl/geo-layers";
import type { Layer } from "@deck.gl/core";
import type { Runtime, SimulationResult } from "../model/types";
import { PROTOCOL } from "../model/constants";

type Point = { position: [number, number]; color: [number, number, number, number]; radius: number; slot: number; label?: string };
const palette = { slate: [92, 111, 139, 150] as [number, number, number, number], blue: [99, 164, 255, 220] as [number, number, number, number], violet: [170, 125, 255, 220] as [number, number, number, number], mint: [121, 239, 191, 240] as [number, number, number, number] };
const pointFor = (region: { longitude: number; latitude: number }, index: number, total: number): [number, number] => [region.longitude + (((index * 17) % 19) - 9) * 0.55 * Math.min(1, 120 / total), region.latitude + (((index * 29) % 13) - 6) * 0.35 * Math.min(1, 120 / total)];

export function NetworkMap({ result, simTime }: { result: SimulationResult; simTime: number }) {
  const [world, setWorld] = useState<GeoJSON.FeatureCollection | null>(null);
  useEffect(() => { fetch(`${import.meta.env.BASE_URL}world.geojson`).then((response) => response.json()).then(setWorld).catch(() => setWorld(null)); }, []);
  const publishedAt = result.events.find((event) => event.type === "BLOCK_PUBLISHED")?.time ?? 0;
  const published = simTime >= publishedAt;
  const logicalReadyAt = useMemo(() => new Map(result.events.filter((event) => event.type === "LOGICAL_READY" && event.slot !== undefined).map((event) => [event.slot!, event.time])), [result]);
  const groupedRepresentative = Math.min(180, Math.max(12, Math.round(result.metrics.groupedSlots / 4)));
  const ordinaryRepresentative = Math.min(180, Math.max(18, Math.round(result.metrics.ordinarySlots / 6)));
  const points = useMemo<Point[]>(() => {
    const built: Point[] = [];
    for (let i = 0; i < groupedRepresentative + ordinaryRepresentative; i += 1) {
      const grouped = i < groupedRepresentative;
      const slot = (i * 31 + (grouped ? 7 : 401)) % PROTOCOL.logicalValidators;
      const region = result.logicalSlots[slot].region;
      built.push({ position: pointFor(region, i, grouped ? groupedRepresentative : ordinaryRepresentative), color: palette.slate, radius: grouped ? 1300 : 1050, slot });
    }
    return built;
  }, [groupedRepresentative, ordinaryRepresentative, result]);
  const styledPoints = points.map((point) => {
    const slot = result.logicalSlots[point.slot];
    const readyAt = logicalReadyAt.get(point.slot) ?? publishedAt + slot.blockReady;
    const state = !published ? "waiting" : simTime >= readyAt ? "ready" : simTime >= publishedAt + slot.blockReady * 0.45 ? "verifying" : "receiving";
    return { ...point, color: state === "ready" ? palette.mint : state === "verifying" ? palette.violet : state === "receiving" ? palette.blue : palette.slate, radius: state === "ready" ? point.radius * 1.45 : point.radius };
  });
  const arcs = result.runtime.sources.flatMap((source, sourceIndex) => result.runtime.provers.map((prover, proverIndex) => ({ source: [source.longitude, source.latitude] as [number, number], target: [prover.longitude, prover.latitude] as [number, number], id: `${sourceIndex}-${proverIndex}` })));
  const blockArcs = result.runtime.producer ? result.logicalSlots.filter((_, index) => index % 34 === 0).map((slot) => ({ source: [result.runtime.producer.longitude, result.runtime.producer.latitude] as [number, number], target: [slot.region.longitude, slot.region.latitude] as [number, number] })) : [];
  const trips = result.logicalSlots.filter((_, index) => index % 24 === 0).map((slot, index) => {
    const source = result.runtime.sources[index % result.runtime.sources.length];
    return { path: [[source.longitude, source.latitude, 0], [slot.region.longitude, slot.region.latitude, result.metrics.workDaTwoThird]] as [number, number, number][], color: [80, 224, 255] };
  });
  const layers: Layer[] = [
    ...(world ? [new GeoJsonLayer({ id: "world", data: world, filled: true, stroked: true, getFillColor: [14, 27, 43, 210], getLineColor: [95, 125, 160, 45], lineWidthMinPixels: 0.35 })] : []),
    new ArcLayer({ id: "da-arcs", data: arcs, getSourcePosition: (d) => d.source, getTargetPosition: (d) => d.target, getSourceColor: [78, 210, 255, 45], getTargetColor: [255, 105, 166, 55], getWidth: 1.2, widthMinPixels: 1 }),
    new TripsLayer({ id: "da-trips", data: trips, getPath: (d: { path: [number, number, number][] }) => d.path, getT: (d: [number, number, number]) => d[2], getColor: (d: { color: [number, number, number] }) => d.color, widthMinPixels: 1.5, trailLength: Math.max(0.8, result.metrics.workDaTwoThird * 0.22), currentTime: simTime, opacity: 0.8 }),
    new ArcLayer({ id: "block-arcs", data: blockArcs, getSourcePosition: (d) => d.source, getTargetPosition: (d) => d.target, getSourceColor: [255, 197, 106, published ? 125 : 25], getTargetColor: [255, 224, 149, published ? 125 : 25], getWidth: 1.4, widthMinPixels: 1 }),
    new ScatterplotLayer({ id: "nodes", data: styledPoints, getPosition: (d) => d.position, getFillColor: (d) => d.color, getRadius: (d) => d.radius, radiusMinPixels: 2, radiusMaxPixels: 10, pickable: false, transitions: { getRadius: 260, getFillColor: 420 } }),
    new ScatterplotLayer({ id: "producer", data: [{ position: [result.runtime.producer.longitude, result.runtime.producer.latitude] }], getPosition: (d) => d.position, getFillColor: [255, 205, 106, 240], getRadius: 2200, radiusMinPixels: 5, radiusMaxPixels: 14 }),
    new ScatterplotLayer({ id: "provers", data: result.runtime.provers.map((region) => ({ position: [region.longitude, region.latitude] })), getPosition: (d) => d.position, getFillColor: [255, 108, 121, 240], getRadius: 1850, radiusMinPixels: 4, radiusMaxPixels: 12 }),
  ];
  return <div className="network-map"><DeckGL initialViewState={{ longitude: 10, latitude: 12, zoom: 0.65, minZoom: 0.45, maxZoom: 2.2 }} controller layers={layers} /><div className="map-vignette" /><div className="map-caption">REPRESENTATIVE NETWORK FIELD <span>•</span> {result.metrics.physicalNodes.toLocaleString()} PHYSICAL NODES</div></div>;
}
