-- Remove continuous aggregate policies first
SELECT remove_continuous_aggregate_policy('uptime_events_daily', if_exists => true);
SELECT remove_continuous_aggregate_policy('uptime_events_hourly', if_exists => true);

-- Drop materialized views (continuous aggregates)
DROP MATERIALIZED VIEW IF EXISTS uptime_events_daily;
DROP MATERIALIZED VIEW IF EXISTS uptime_events_hourly;

-- Remove compression policy before dropping table
SELECT remove_compression_policy('uptime_events', if_exists => true);

-- Drop the hypertable (this also drops indexes)
DROP TABLE IF EXISTS uptime_events;
