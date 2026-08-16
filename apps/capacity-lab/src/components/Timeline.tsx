import { Check, Circle, Database, FileCheck2, Gauge, Layers3, Network, ShieldCheck, Sparkles, type LucideIcon } from "lucide-react";
import { motion } from "motion/react";
import type { Language } from "../i18n";
import { copy } from "../i18n";
import type { SimulationEvent, SimulationResult } from "../model/types";

type Step = { label: string; icon: LucideIcon; at: number };

export function Timeline({ language, result, simTime }: { language: Language; result: SimulationResult; simTime: number }) {
  const t = copy[language];
  const m = result.metrics;
  const timeAt = (type: SimulationEvent["type"], fallback: number) => result.events.find((event) => event.type === type)?.time ?? fallback;
  const publishedAt = timeAt("BLOCK_PUBLISHED", 0);
  const quorumAt = timeAt("QUORUM_2_3", m.roundTimeSeconds);
  const ready = result.events.filter((event) => event.type === "LOGICAL_READY" && event.time <= simTime).length;
  const reportAt = timeAt("REPORT_READY", m.reportReadySeconds);
  const work: Step[] = [
    { label: t.refine, icon: Gauge, at: timeAt("WORK_START", 0) },
    { label: t.prove, icon: ShieldCheck, at: timeAt("PROOF_START", 0) },
    { label: t.aggregate, icon: Layers3, at: reportAt - 0.3 },
    { label: t.report, icon: FileCheck2, at: reportAt },
    { label: t.gate, icon: Sparkles, at: reportAt },
  ];
  const block: Step[] = [
    { label: t.published, icon: Circle, at: publishedAt },
    { label: t.propagate, icon: Network, at: timeAt("BLOCK_DATA_READY", publishedAt) },
    { label: t.verify, icon: Database, at: timeAt("BLOCK_VERIFY_READY", publishedAt) },
    { label: t.consensus, icon: ShieldCheck, at: quorumAt },
    { label: t.ready, icon: Check, at: quorumAt },
  ];
  return <div className="timeline-stack"><Lane title={t.work} elapsed={simTime} total={reportAt} steps={work} /><Lane title={t.block} elapsed={simTime} total={quorumAt} steps={block} suffix={`${ready.toLocaleString()} / 1,023 ready`} /></div>;
}

function Lane({ title, elapsed, total, steps, suffix }: { title: string; elapsed: number; total: number; steps: Step[]; suffix?: string }) {
  return <section className="timeline-lane"><div className="lane-heading"><span>{title}</span><b>{suffix ?? `T+${elapsed.toFixed(2)} s / ${total.toFixed(2)} s`}</b></div><div className="step-track"><div className="track-fill" style={{ width: `${Math.min(100, (elapsed / Math.max(0.001, total)) * 100)}%` }} />{steps.map((step, index) => { const done = elapsed >= step.at; const active = !done && elapsed >= (steps[index - 1]?.at ?? 0); return <motion.div key={step.label} className={`timeline-step ${done ? "done" : active ? "active" : ""}`} layout><div className="step-icon"><step.icon size={14} /></div><span>{step.label}</span><small>{done ? "complete" : `${step.at.toFixed(2)} s`}</small></motion.div>; })}</div></section>;
}
