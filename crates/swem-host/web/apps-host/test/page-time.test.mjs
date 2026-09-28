// A schedule's time, as a person chooses it and reads it.

import assert from "node:assert/strict";
import { test } from "node:test";

import { moment, ran, took, until, whenOf } from "../src/page/time.ts";

const chosen = { how: "daily", every: 30, unit: "minutes", at: "06:05", day: "1", once: "2026-09-28T10:00", line: "0 9 * * 1-5" };

test("what a person chose is the moment the host keeps", () => {
  assert.deepEqual(whenOf(chosen, "Europe/Kyiv"), { kind: "cron", line: "5 6 * * *", zone: "Europe/Kyiv" });
  assert.deepEqual(whenOf({ ...chosen, how: "weekly" }, "Europe/Kyiv"), { kind: "cron", line: "5 6 * * 1", zone: "Europe/Kyiv" });
  assert.deepEqual(whenOf({ ...chosen, how: "every", every: 2, unit: "hours" }, "UTC"), { kind: "every", minutes: 120 });
  assert.deepEqual(whenOf({ ...chosen, how: "every", every: 1, unit: "days" }, "UTC"), { kind: "every", minutes: 1440 });
  assert.deepEqual(whenOf({ ...chosen, how: "cron", line: " 0 9 * * 1-5 " }, "UTC"), { kind: "cron", line: "0 9 * * 1-5", zone: "UTC" });
  assert.equal(whenOf({ ...chosen, how: "once" }, "UTC").at_ms, new Date("2026-09-28T10:00").getTime());
});

test("how long until it is due, and how a run went, are said as a person says them", () => {
  const now = new Date("2026-09-28T10:00:00").getTime();
  assert.equal(until(now + 20_000, now), "in under a minute");
  assert.equal(until(now + 12 * 60_000, now), "in 12 min");
  assert.equal(until(now + 7 * 3_600_000, now), "in 7 h");
  assert.equal(until(now + 5 * 86_400_000, now), "in 5 days");
  assert.equal(until(now - 1, now), "now");
  assert.equal(took(41_000), "41 s");
  assert.ok(moment(now - 4 * 3_600_000, now).startsWith("today "));
  assert.ok(moment(now - 28 * 3_600_000, now).startsWith("yesterday "));
  const run = { schedule_id: "s", say: "check", due_ms: now - 3_600_000, claimed_ms: now - 3_600_000 + 1_000, ended_ms: now - 3_600_000 + 42_000, late: false, state: "answered" };
  // The time of day is said the way the person's own system says it.
  assert.match(ran(run, now), /^today \d\d:\d\d( [AP]M)? · 41 s$/);
  assert.match(ran({ ...run, late: true, claimed_ms: now - 60_000, ended_ms: now - 54_000 }, now), /nobody kept time then, said at \d\d:\d\d( [AP]M)? · 6 s$/);
  assert.match(ran({ ...run, state: "skipped", note: "the run before it had not ended" }, now), /· the run before it had not ended$/);
});
