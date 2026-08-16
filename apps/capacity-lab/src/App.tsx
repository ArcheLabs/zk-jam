import { useEffect, useMemo, useRef, useState } from "react";
import { Activity, Globe2, Zap } from "lucide-react";
import { Controls } from "./components/Controls";
import { Metrics } from "./components/Metrics";
import { StatusPanel } from "./components/StatusPanel";
import { Timeline } from "./components/Timeline";
import { Badge, Card } from "./components/ui";
import { copy, type Language } from "./i18n";
import { formatGas } from "./model/formulas";
import { buildScenario } from "./sim/scenario";
import { NetworkMap } from "./map/NetworkMap";
import type { Parameters } from "./model/types";

const initial: Parameters = { groupShare: 0.3, nodesPerGroup: 30, ethBlocksPerItem: 1, networkLoad: 0.7 };
export default function App() {
  const [language, setLanguage] = useState<Language>("en"); const [draft, setDraft] = useState(initial); const [params, setParams] = useState(initial); const [seed, setSeed] = useState(20260816); const [simTime, setSimTime] = useState(0); const [playing, setPlaying] = useState(true); const [history, setHistory] = useState<number[]>([]); const lastFrame = useRef<number | null>(null); const t = copy[language];
  const result = useMemo(() => buildScenario(params, seed), [params, seed]);
  useEffect(() => { if (!playing) return; let frame = 0; const tick = (now: number) => { if (lastFrame.current == null) lastFrame.current = now; const delta = (now - lastFrame.current) / 1000; lastFrame.current = now; setSimTime((current) => { const next = current + delta * 4; if (next >= result.metrics.effectiveInterval) { setHistory((items) => [...items.slice(-29), result.metrics.effectiveInterval]); setSeed((value) => value + 1); return 0; } return next; }); frame = requestAnimationFrame(tick); }; frame = requestAnimationFrame(tick); return () => { cancelAnimationFrame(frame); lastFrame.current = null; }; }, [playing, result.metrics.effectiveInterval]);
  const average = history.length ? history.reduce((sum, item) => sum + item, 0) / history.length : result.metrics.effectiveInterval;
  const apply = () => { setParams(draft); setSimTime(0); setPlaying(true); setSeed((value) => value + 1); };
  const reroll = () => { setSimTime(0); setSeed((value) => value + 1); };
  return <div className="app-shell"><header className="topbar"><div className="brand"><div className="brand-mark"><Zap size={18} /></div><div><h1>ZkJAM Capacity Lab</h1><p>{t.subtitle}</p></div></div><div className="topbar-right"><Badge className="research-badge"><Activity size={11} /> RESEARCH INSTRUMENT</Badge><select aria-label="Language" value={language} onChange={(event) => setLanguage(event.target.value as Language)}><option value="en">English</option><option value="zh">中文</option><option value="ja">日本語</option><option value="fr">Français</option></select></div></header><Controls language={language} value={draft} pending={JSON.stringify(draft) !== JSON.stringify(params)} onChange={setDraft} onApply={apply} onReroll={reroll} /><main className="stage"><NetworkMap result={result} simTime={simTime} /><div className="stage-glow" /><div className="hero-metrics"><Hero label={t.physical} value={result.metrics.physicalNodes.toLocaleString()} meta={`${result.metrics.groupedSlots.toLocaleString()} grouped · 1,023 logical weights`} accent="blue" /><Hero label={t.interval} value={`${result.metrics.effectiveInterval.toFixed(2)} s`} meta={`avg ${average.toFixed(2)} s · ${history.length || 1} sample${history.length === 1 ? "" : "s"}`} accent="mint" /></div><div className="map-legend"><span><i className="dot dot-da" />DA</span><span><i className="dot dot-proof" />PROOF</span><span><i className="dot dot-ready" />READY</span><span><i className="dot dot-producer" />PRODUCER</span></div><div className="right-rail"><Metrics language={language} result={result} /><StatusPanel language={language} result={result} simTime={simTime} playing={playing} onToggle={() => setPlaying((value) => !value)} onReroll={reroll} /></div><div className="timeline-wrap"><Timeline language={language} result={result} simTime={simTime} /></div><Card className="model-note"><div className="eyebrow"><Globe2 size={11} /> MODEL NOTE</div><p>{t.mapNote}</p><div className="note-row"><span>ETH-block eq / item</span><strong>{result.params.ethBlocksPerItem.toFixed(2)}</strong><span>≈ {formatGas(result.metrics.gasEquivalent)} gas</span></div></Card></main></div>;
}
function Hero({ label, value, meta, accent }: { label: string; value: string; meta: string; accent: string }) { return <Card className={`hero-card ${accent}`}><span>{label}</span><strong>{value}</strong><small>{meta}</small></Card>; }
