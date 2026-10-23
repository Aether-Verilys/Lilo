/** 本地日期工具。全部按本地时区计算，不用 UTC —— 「今天」是用户的今天。 */

export function todayStr(): string {
  return fmt(new Date());
}

export function fmt(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export function parse(s: string): Date {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
}

export function addDays(s: string, n: number): string {
  const d = parse(s);
  d.setDate(d.getDate() + n);
  return fmt(d);
}

const WEEK = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

/** 「今天 周一」「明天 周二」「9月10日 周三」 */
export function label(s: string | null): string {
  if (!s) return "收集箱";
  const today = todayStr();
  const w = WEEK[parse(s).getDay()];
  if (s === today) return `今天 ${w}`;
  if (s === addDays(today, 1)) return `明天 ${w}`;
  if (s === addDays(today, -1)) return `昨天 ${w}`;
  const d = parse(s);
  return `${d.getMonth() + 1}月${d.getDate()}日 ${w}`;
}

export function isToday(s: string | null): boolean {
  return s === todayStr();
}

export function isPast(s: string | null): boolean {
  return s !== null && s < todayStr();
}

/** 含 from 与 to 的日期序列 */
export function range(from: string, to: string): string[] {
  const out: string[] = [];
  let cur = from;
  while (cur <= to) {
    out.push(cur);
    cur = addDays(cur, 1);
  }
  return out;
}

/** 某月第一天 / 最后一天 */
export function monthBounds(s: string): [string, string] {
  const d = parse(s);
  const first = new Date(d.getFullYear(), d.getMonth(), 1);
  const last = new Date(d.getFullYear(), d.getMonth() + 1, 0);
  return [fmt(first), fmt(last)];
}

export function timeStr(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(
    d.getMinutes()
  ).padStart(2, "0")}`;
}
