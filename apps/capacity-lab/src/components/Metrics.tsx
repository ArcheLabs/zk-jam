import { Info } from "lucide-react";
import { motion } from "motion/react";
import { Badge, Card } from "./ui";
import type { Language } from "../i18n";
import { copy } from "../i18n";
import type { SimulationResult } from "../model/types";
import { formatGas } from "../model/formulas";

const seconds = (value: number | null) => value == null ? "—" : value < 1 ? `${Math.round(value * 1000)} ms` : `${value.toFixed(value < 10 ? 2 : 1)} s`;
export function Metrics({ language, result }: { language: Language; result: SimulationResult }) {
  const t = copy[language]; const m = result.metrics;
  const item = (label: string, value: string, source: string) => <div className="metric-row"><span>{label}<Info size={11} /></span><strong>{value}</strong><Badge>{source}</Badge></div>;
  return <Card className="metrics-card"><div className="card-heading"><div><div className="eyebrow">{t.metrics}</div><div className="card-subheading">{result.runtime.provers.length} provers · {result.runtime.producer.name} producer</div></div><span className="live-dot">LIVE</span></div>{item(t.gas, formatGas(m.gasEquivalent), "DERIVED")}{item(t.proof, seconds(m.proofAvg), "OPENVM + DERIVED")}{item(t.reportMetric, seconds(m.workReport), "DERIVED")}{item(t.da, seconds(m.workDaTwoThird), "JAM MODEL")}{item(t.blockMetric, seconds(m.blockTwoThird), "DERIVED")}{item(t.groupReady, seconds(m.groupReadyP50), "DERIVED")}{item(t.ordinaryReady, seconds(m.ordinaryReadyP50), "DERIVED")}{item(t.smallDa, `${m.smallNodeDaMbps.toFixed(1)} Mbps`, "DERIVED")}{item(t.clusters, `~${Math.ceil(m.requiredClusters).toLocaleString()}`, "DERIVED")}{m.gasEquivalent > 5e9 && <Badge className="stress-badge">{t.above}</Badge>}<div className="metrics-foot"><span>Group slots <b>{m.groupedSlots.toLocaleString()}</b></span><span>Ordinary <b>{m.ordinarySlots.toLocaleString()}</b></span><span>Load <b>{Math.round(result.params.networkLoad * 100)}%</b></span></div><motion.div className="pressure" animate={{ opacity: [0.65, 1, 0.65] }} transition={{ duration: 2.8, repeat: Infinity }}><span>PROVER THROUGHPUT STRESS</span><strong className={m.pressure === "PROVER-BOUND" ? "danger" : "ok"}>{m.pressure}</strong></motion.div></Card>;
}
