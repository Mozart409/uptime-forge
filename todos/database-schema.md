# Timescale DB Schema

```sql
-- Main events table with error tracking
CREATE TABLE uptime_events (
  endpoint_id uuid NOT NULL,
  ts timestamptz NOT NULL,
  status_code int,
  success boolean NOT NULL,
  latency_ms int,
  error_type text,           -- 'timeout', 'dns', 'tls', 'connection', 'status_mismatch', etc.
  error_message text,        -- Detailed error message for failed checks
  PRIMARY KEY (endpoint_id, ts)
);

-- Convert to hypertable with 7-day chunks
SELECT create_hypertable('uptime_events', 'ts', chunk_time_interval => INTERVAL '7 days');

-- Enable compression for older data
ALTER TABLE uptime_events SET (
  timescaledb.compress,
  timescaledb.compress_orderby = 'ts DESC',
  timescaledb.compress_segmentby = 'endpoint_id'
);
SELECT add_compression_policy('uptime_events', INTERVAL '30 days');

-- Hourly rollup for faster queries
CREATE MATERIALIZED VIEW uptime_events_hourly
WITH (timescaledb.continuous) AS
SELECT
  endpoint_id,
  time_bucket(INTERVAL '1 hour', ts) AS hour,
  count(*) AS checks,
  sum(success::int) AS successes,
  avg(latency_ms) AS avg_latency_ms,
  max(latency_ms) AS max_latency_ms,
  percentile_cont(0.95) WITHIN GROUP (ORDER BY latency_ms) AS p95_latency_ms,
  percentile_cont(0.99) WITHIN GROUP (ORDER BY latency_ms) AS p99_latency_ms
FROM uptime_events
GROUP BY endpoint_id, hour;

SELECT add_continuous_aggregate_policy('uptime_events_hourly',
  start_offset => INTERVAL '30 days',
  end_offset   => INTERVAL '1 hour',
  schedule_interval => INTERVAL '5 minutes');

-- Daily rollup for long-term trends
CREATE MATERIALIZED VIEW uptime_events_daily
WITH (timescaledb.continuous) AS
SELECT
  endpoint_id,
  time_bucket(INTERVAL '1 day', ts) AS day,
  count(*) AS checks,
  sum(success::int) AS successes,
  avg(latency_ms) AS avg_latency_ms,
  max(latency_ms) AS max_latency_ms,
  percentile_cont(0.95) WITHIN GROUP (ORDER BY latency_ms) AS p95_latency_ms,
  percentile_cont(0.99) WITHIN GROUP (ORDER BY latency_ms) AS p99_latency_ms
FROM uptime_events
GROUP BY endpoint_id, day;

SELECT add_continuous_aggregate_policy('uptime_events_daily',
  start_offset => INTERVAL '365 days',
  end_offset   => INTERVAL '1 day',
  schedule_interval => INTERVAL '1 hour');

-- Index for querying by error type
CREATE INDEX idx_uptime_events_error_type ON uptime_events (endpoint_id, error_type, ts DESC)
  WHERE error_type IS NOT NULL;
```

## Error Types

The `error_type` column uses the following values:

| Error Type | Description |
|------------|-------------|
| `timeout` | Request timed out |
| `dns` | DNS resolution failed |
| `tls` | TLS/SSL certificate error |
| `connection` | Connection refused or reset |
| `status_mismatch` | HTTP status code didn't match expected |
| `tcp_refused` | TCP connection refused |
| `dns_nxdomain` | Domain does not exist |
| `dns_mismatch` | DNS records didn't match expected |
