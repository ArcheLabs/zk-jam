import { RotateCcw, SlidersHorizontal } from "lucide-react";
import { Badge, Button, Card, FieldLabel, Slider } from "./ui";
import { copy, type Language } from "../i18n";
import type { Parameters } from "../model/types";

export function Controls({ language, value, pending, onChange, onApply, onReroll }: { language: Language; value: Parameters; pending: boolean; onChange: (value: Parameters) => void; onApply: () => void; onReroll: () => void }) {
  const t = (key: keyof typeof copy.en) => copy[language][key];
  return <aside className="controls"><div className="controls-title"><div><SlidersHorizontal size={14} /><span>{t("parameters")}</span></div>{pending && <Badge className="pending-badge">{t("pending")}</Badge>}</div>
    <Card className="control-card"><div className="eyebrow">{t("core")}</div>
      <Parameter label={t("groupShare")} value={`${Math.round(value.groupShare * 100)}%`} hint={t("groupHint")}><Slider min={0} max={100} value={value.groupShare * 100} onChange={(event) => onChange({ ...value, groupShare: Number(event.target.value) / 100 })} /></Parameter>
      <Parameter label={t("nodes")} value={`${value.nodesPerGroup}`} hint={t("nodesHint")}><Slider min={1} max={300} value={value.nodesPerGroup} onChange={(event) => onChange({ ...value, nodesPerGroup: Number(event.target.value) })} /></Parameter>
      <Parameter label={t("workload")} value={`${value.ethBlocksPerItem.toFixed(value.ethBlocksPerItem < 10 ? 2 : 1)} ETH blocks`} hint={t("workloadHint")}><input className="number-input" type="number" min="0.25" max="500" step="0.25" value={value.ethBlocksPerItem} onChange={(event) => onChange({ ...value, ethBlocksPerItem: Math.min(500, Math.max(0.25, Number(event.target.value) || 0.25)) })} /></Parameter>
      <Parameter label={t("load")} value={`${Math.round(value.networkLoad * 100)}%`} hint={t("loadHint")}><Slider min={1} max={100} value={value.networkLoad * 100} onChange={(event) => onChange({ ...value, networkLoad: Number(event.target.value) / 100 })} /></Parameter>
    </Card>
    <Card className="reference-card"><div className="eyebrow">{t("smallReference")}</div><div className="reference-name">Constrained home node</div><div className="reference-grid"><span>WAN</span><strong>100↓ / 20↑ Mbps</strong><span>CPU</span><strong>4 cores</strong><span>Overlay RTT</span><strong>20–30 ms</strong><span>Replication</span><strong>3× copies</strong><span>Verify</span><strong>2 workers / node</strong></div><div className="source-line">{t("source")}: ZKJAM DESIGN</div></Card>
    <div className="control-actions"><Button className="primary-button" onClick={onApply}>{t("apply")}</Button><Button className="secondary-button" onClick={onReroll}><RotateCcw size={13} />{t("reroll")}</Button></div>
  </aside>;
}

function Parameter({ label, value, hint, children }: { label: string; value: string; hint: string; children: React.ReactNode }) { return <div className="parameter"><div className="parameter-head"><FieldLabel>{label}</FieldLabel><strong>{value}</strong></div>{children}<p>{hint}</p></div>; }
