import { Info } from "lucide-react";
import { motion } from "motion/react";
import { Badge, Card } from "./ui";
import type { Language } from "../i18n";
import { copy } from "../i18n";
import type { SimulationResult } from "../model/types";

const seconds = (value: number | null) => value == null ? "—" : value < 1 ? `${Math.round(value * 1000)} ms` : `${value.toFixed(value < 10 ? 2 : 1)} s`;
export function Metrics({ language, result }: { language: Language; result: SimulationResult }) {
  const t = copy[language]; const m = result.metrics;
  const item = (label: string, value: string, source: string) => <div className="metric-row"><span>{label}<Info size={11} /></span><strong>{value}</strong><Badge>{source}</Badge></div>;
  return <Card className="metrics-card"><div className="card-heading"><div><div className="eyebrow">{t.metrics}</div><div className="card-subheading">{result.runtime.provers.length} provers · {result.runtime.producer.name} producer</div></div><span className="live-dot">LIVE</span></div>{item(t.proof, seconds(m.proofBarrierSeconds), "OPENVM + DERIVED")}{item(t.reportMetric, seconds(m.reportReadySeconds), "DERIVED")}{item(t.blockMetric, seconds(m.blockToTwoThirdSeconds), "DERIVED")}{item(t.logicalP99, seconds(m.logicalReadyP99), "DERIVED")}{item(t.groupReady, seconds(m.groupReadyP50), "DERIVED")}{item(t.ordinaryReady, seconds(m.ordinaryReadyP50), "DERIVED")}{item(t.daNode, `${m.daStoredPerNodeMb.toFixed(2)} MB`, "DERIVED")}{item(t.packagesNode, `${m.workPackagesPerNode.toFixed(2)}`, "DERIVED")}{item(t.groupCommunication, seconds(m.groupCommunicationSeconds), "DERIVED")}<motion.div className="pressure" animate={{ opacity: [0.65, 1, 0.65] }} transition={{ duration: 2.8, repeat: Infinity }}><span>{t.dominant}</span><strong className="ok">{m.dominantBottleneck}</strong></motion.div></Card>;
}
