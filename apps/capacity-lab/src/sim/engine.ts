export type Resource = { id: string; capacityMbps: number };
export type Flow = { id: string; remainingMB: number; resources: Resource[] };
export type FlowCompletion = { id: string; time: number };

/** Small max-min fluid scheduler. It advances only to the next flow completion. */
export function runFluidFlows(flows: Flow[]): FlowCompletion[] {
  const active = flows.map((flow) => ({ ...flow, resources: [...flow.resources] }));
  const completed: FlowCompletion[] = [];
  let clock = 0;
  while (active.length) {
    const shares = new Map<string, number>();
    for (const flow of active) for (const resource of flow.resources) shares.set(resource.id, (shares.get(resource.id) ?? 0) + 1);
    const rates = active.map((flow) => {
      const rate = Math.min(...flow.resources.map((resource) => resource.capacityMbps / (shares.get(resource.id) ?? 1))) / 8;
      return Math.max(0.0001, rate);
    });
    const next = Math.min(...active.map((flow, index) => flow.remainingMB / rates[index]));
    clock += next;
    for (let index = active.length - 1; index >= 0; index -= 1) {
      active[index].remainingMB -= rates[index] * next;
      if (active[index].remainingMB <= 0.00001) {
        completed.push({ id: active[index].id, time: clock });
        active.splice(index, 1);
        rates.splice(index, 1);
      }
    }
  }
  return completed;
}
