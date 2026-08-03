# Retention policy + continuous-aggregate offsets

Status: **proposed, not applied.** Measured against the live homelab instance
(`containers` host, TimescaleDB 2.25.0 / PG 16.11) on 2026-07-31.

## Problem

`uptime_events` has **no retention policy** — it grows forever. Today it is
130 MB (24 chunks, ~465k rows, oldest data ~2026-02-13) on a VM whose root
filesystem has 7.1 GB free.

The obvious fix — `add_retention_policy('uptime_events', INTERVAL '90 days')` —
is a **data-loss trap** as the schema currently stands.

### Why a naive retention policy destroys the daily rollup

`migrations/0002_uptime_events.up.sql:61` sets the daily aggregate's refresh
window to a full year:

```sql
add_continuous_aggregate_policy ('uptime_events_daily',
    start_offset => INTERVAL '365 days', ...)
```

Every refresh re-materializes any *invalidated* bucket within `now() - 365 days`.
If retention has dropped the raw chunks under part of that window, the recompute
returns zero rows and TimescaleDB **replaces the existing materialized rows with
nothing** — silently deleting long-term history, which is the one thing the daily
rollup exists to preserve.

The rule (documented by Timescale) is:

> refresh `start_offset`  <  retention `drop_after`

Currently `start_offset` is 365 days and `drop_after` is infinity, so the
invariant holds only by accident. Any retention shorter than a year breaks it.

Both aggregates are `materialized_only = true`, so *reads* of
`uptime_events_hourly` / `uptime_events_daily` never touch raw data. Only
refreshes are affected. That is what makes shrinking the offsets safe.

## Decision

Raw samples are append-only and written at ~`now()` — uptime-forge inserts each
result immediately after the check, so there are no late arrivals. The refresh
windows only need to cover a realistic backfill/outage gap, not the full
retention horizon.

| Knob | Current | Proposed | Rationale |
|---|---|---|---|
| `uptime_events_hourly` `start_offset` | 30 days | **7 days** | covers a week-long outage backfill |
| `uptime_events_daily` `start_offset` | 365 days | **30 days** | covers a month; older daily buckets are frozen |
| `uptime_events` retention | none | **90 days** | 3× the largest `start_offset` |
| `uptime_events` `chunk_time_interval` | 7 days | **1 day** | see below |
| compression `compress_after` | 30 days | **2 days** | see below |

`end_offset` stays at 1 hour / 1 day for hourly / daily.

### Why chunk_interval and compression move too

Compression is getting ~30× (a 33 MB chunk becomes ~950 kB), but
`compress_after = 30 days` against 7-day chunks means **4 chunks stay
uncompressed — 106 MB of the 130 MB total.** Retention alone barely helps,
because you always carry two or three fat uncompressed chunks.

Shrinking chunks to 1 day makes compression actually bite. Measured write rate
is ~4.7 MB/day uncompressed, ~0.16 MB/day compressed.

| | today | retention only | retention + 1-day chunks |
|---|---|---|---|
| uncompressed | 106 MB | ~66–99 MB | ~14 MB (3 chunks) |
| compressed | 23 MB | ~11 MB | ~14 MB (87 chunks) |
| aggregates | 6.5 MB | 6.5 MB | 6.5 MB |
| **total** | **130 MB, unbounded** | **~85–115 MB** | **~35 MB, bounded** |

`compress_after` must exceed `chunk_time_interval` so the open chunk is never
targeted; 2 days against 1-day chunks is safe.

## Migration

Ordering matters: shrink the refresh windows **before** adding retention, so no
refresh is ever in flight over a range that is about to be dropped. Shrinking a
window never deletes already-materialized buckets — it just stops revisiting
them — so steps 1–2 are non-destructive.

```sql
-- 1. Narrow the refresh windows first.
SELECT remove_continuous_aggregate_policy('uptime_events_hourly');
SELECT add_continuous_aggregate_policy('uptime_events_hourly',
    start_offset      => INTERVAL '7 days',
    end_offset        => INTERVAL '1 hour',
    schedule_interval => INTERVAL '5 minutes');

SELECT remove_continuous_aggregate_policy('uptime_events_daily');
SELECT add_continuous_aggregate_policy('uptime_events_daily',
    start_offset      => INTERVAL '30 days',
    end_offset        => INTERVAL '1 day',
    schedule_interval => INTERVAL '1 hour');

-- 2. Smaller chunks (new chunks only; the existing 7-day chunks age out
--    under the retention policy within ~3 months).
SELECT set_chunk_time_interval('uptime_events', INTERVAL '1 day');

-- 3. Compress aggressively.
SELECT remove_compression_policy('uptime_events');
SELECT add_compression_policy('uptime_events', INTERVAL '2 days');

-- 4. Retention last, now that 90 days > 30-day start_offset.
SELECT add_retention_policy('uptime_events', INTERVAL '90 days');
```

For the repo this should land as `migrations/0003_retention_policy.up.sql`, with
a `.down.sql` restoring the 0002 values. `add_*_policy` calls are not idempotent
against an existing policy — the `remove_*` calls above handle that, but a fresh
database created from 0002 + 0003 will run them against policies that exist, so
the removes are required, not optional.

## Verify after applying

```sql
SELECT job_id, proc_name, config FROM timescaledb_information.jobs
 WHERE proc_name <> 'policy_telemetry';
-- expect: refresh(hourly)=7d/1h, refresh(daily)=30d/1d,
--         compression=2d, retention=90d

SELECT count(*) FILTER (WHERE is_compressed) AS compressed, count(*) AS total
  FROM timescaledb_information.chunks WHERE hypertable_name = 'uptime_events';
-- compressed/total should climb toward ~87/90 over the next few days

SELECT min(day), max(day), count(*) FROM uptime_events_daily;
-- min(day) must NOT move forward after the retention job first runs.
--   If it does, the invariant was violated and daily history is being eaten.
```

The daily-rollup check is the important one — run it before applying and again
after the first retention job fires (`policy_retention` is scheduled every
12 hours by default).

## Open questions

- **Retention on the aggregates themselves?** `uptime_events_hourly` is 6 MB
  after ~5.5 months (168 rows/day) — roughly 65 MB over 5 years. Probably worth
  `add_retention_policy('uptime_events_hourly', INTERVAL '2 years')`.
  `uptime_events_daily` is 488 kB and should be kept forever. Not included above
  because it is a product decision, not a correctness fix.
- **`end_offset` makes the current bucket invisible.** Because both aggregates
  are `materialized_only = true`, `uptime_events_hourly` never shows the
  in-progress hour and `uptime_events_daily` never shows today. If the UI reads
  from the rollups, "last hour" will always look empty. Unrelated to retention,
  but worth confirming it is intentional rather than a reporting gap.

## Doc drift spotted

`todos/database-schema.md:6` declares `endpoint_id uuid NOT NULL`, but
`migrations/0002_uptime_events.up.sql:3` and the live database both use `text`.
The schema doc also still shows the pre-`WITH NO DATA` aggregate definitions.
Worth reconciling so the doc is not read as authoritative.
