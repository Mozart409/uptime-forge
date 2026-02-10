# Timescale DB Schema


```sql

CREATE TABLE events (
  endpoint_id uuid NOT NULL,
  ts timestamptz NOT NULL,
  status_code int,
  success boolean NOT NULL,
  latency_ms int,
  PRIMARY KEY (endpoint_id, ts)
);
SELECT create_hypertable('uptime_events', 'ts', chunk_time_interval => INTERVAL '7 days');
ALTER TABLE events SET (
  timescaledb.compress,
  timescaledb.compress_orderby = 'ts DESC',
  timescaledb.compress_segmentby = 'endpoint_id'
);
SELECT add_compression_policy('uptime_events', INTERVAL '30 days');
CREATE MATERIALIZED VIEW events_daily
WITH (timescaledb.continuous) AS
SELECT
  endpoint_id,
  time_bucket(INTERVAL '1 day', ts) AS day,
  count(*) AS checks,
  sum(success::int) AS successes,
  avg(latency_ms) AS avg_latency_ms,
  max(latency_ms) AS max_latency_ms
FROM events
GROUP BY endpoint_id, day;
SELECT add_continuous_aggregate_policy('uptime_events_daily',
  start_offset => INTERVAL '365 days',
  end_offset   => INTERVAL '1 day',
  schedule_interval => INTERVAL '1 hour');

```

