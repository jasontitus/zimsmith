import { useEffect, useRef, useState } from "react";

export function progressCount(
  completed: number,
  total: number | null,
  unit: string,
) {
  const amount = (n: number) =>
    unit === "bytes" ? `${(n / 1024 ** 2).toFixed(1)} MB` : n.toLocaleString();
  if (total != null && total > 0)
    return `${amount(completed)} of ${amount(total)}${unit === "bytes" ? "" : ` ${unit}`}`;
  return completed > 0
    ? `${amount(completed)}${unit === "bytes" ? "" : ` ${unit}`}`
    : "";
}

function duration(seconds: number) {
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min`;
  return `${Math.floor(minutes / 60)} hr ${minutes % 60} min`;
}

export function useProgressTiming(
  step: string | null,
  completed: number,
  total: number | null,
) {
  const [now, setNow] = useState(() => Date.now() / 1000);
  const history = useRef<{
    key: string;
    started: number;
    samples: { time: number; completed: number }[];
  }>(null);
  const key = step && JSON.stringify([step, total]);
  useEffect(() => {
    if (!key) return;
    const timer = setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => clearInterval(timer);
  }, [key]);
  useEffect(() => {
    if (!key) {
      history.current = null;
      return;
    }
    const time = Date.now() / 1000;
    let current = history.current;
    if (current?.key !== key || completed < current.samples.at(-1)!.completed) {
      current = { key, started: time, samples: [{ time, completed }] };
      history.current = current;
    } else if (completed > current.samples.at(-1)!.completed) {
      current.samples.push({ time, completed });
      while (current.samples.length > 2 && current.samples[1].time < time - 20)
        current.samples.shift();
    }
  }, [key, completed]);
  const current = history.current;
  if (!key || current?.key !== key) return { now, timing: "" };
  const elapsed = duration(Math.max(0, Math.floor(now - current.started)));
  const first = current.samples[0];
  const last = current.samples.at(-1)!;
  const observed = last.time - first.time;
  if (
    total != null &&
    total > completed &&
    observed >= 3 &&
    current.samples.length >= 3 &&
    now - last.time <= 10
  ) {
    const seconds = Math.max(
      5,
      Math.ceil(
        ((total - completed) * observed) /
          (last.completed - first.completed) /
          5,
      ) * 5,
    );
    return {
      now,
      timing: `About ${duration(seconds)} left in this step · ${elapsed} elapsed`,
    };
  }
  return {
    now,
    timing: `${elapsed} in this step · ${total != null && total > completed && now - last.time <= 10 ? "Estimating time left…" : "No time estimate yet"}`,
  };
}
