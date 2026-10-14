const UNITS: [limitSeconds: number, seconds: number, unit: string][] = [
  [60, 1, "second"],
  [3600, 60, "minute"],
  [86_400, 3600, "hour"],
  [Number.POSITIVE_INFINITY, 86_400, "day"],
];

/** "just now", "5 minutes ago", "3 days ago". */
export function timeAgo(date: Date, now: Date): string {
  const seconds = Math.max(0, Math.round((now.getTime() - date.getTime()) / 1000));
  if (seconds < 5) return "just now";
  const [, size, unit] = UNITS.find(([limit]) => seconds < limit) ?? UNITS[UNITS.length - 1];
  const n = Math.floor(seconds / size);
  return `${n} ${unit}${n === 1 ? "" : "s"} ago`;
}
