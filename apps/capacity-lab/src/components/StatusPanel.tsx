import { Activity, Pause, Play, Radio, Shuffle } from "lucide-react";
import { Button, Card } from "./ui";
import { copy, type Language } from "../i18n";
import type { SimulationResult } from "../model/types";

export function StatusPanel({ language, result, simTime, playing, onToggle, onReroll }: { language: Language; result: SimulationResult; simTime: number; playing: boolean; onToggle: () => void; onReroll: () => void }) {
  const t = copy[language]; const m = result.metrics; const ready = result.events.filter((event) => event.type === "LOGICAL_READY" && event.time <= simTime).length; const progress = ready / 1023;
  return <Card className="status-panel"><div className="status-head"><div><div className="eyebrow">{t.legend}</div><strong><Radio size={13} /> SLOT #{result.runtime.seed % 10_000}</strong></div><Button className="icon-button" onClick={onToggle} aria-label="Toggle playback">{playing ? <Pause size={14} /> : <Play size={14} />}</Button></div><div className="status-time"><span>SIMULATION PLAYHEAD</span><b>{simTime.toFixed(2)} s</b></div><div className="progress-label"><span>Logical validators ready</span><strong>{ready.toLocaleString()} / 1,023</strong></div><div className="progress"><MotionFill value={progress} /></div><div className="status-grid"><div><span>Block → 2/3</span><b>{m.blockTwoThird.toFixed(2)} s</b></div><div><span>Proof gate</span><b>{m.proofGate.toFixed(2)} s</b></div><div><span>Active cores</span><b>{m.activeCores}</b></div><div><span>Topology</span><b>{result.runtime.producer.name}</b></div></div><Button className="reroll-link" onClick={onReroll}><Shuffle size={12} /> {t.reroll}</Button></Card>;
}
function MotionFill({ value }: { value: number }) { return <div className="progress-fill" style={{ width: `${Math.min(100, value * 100)}%` }} />; }
