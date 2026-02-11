-- TimescaleDB for time-series data
CREATE EXTENSION IF NOT EXISTS timescaledb;

-- Query performance monitoring (built-in, just needs enabling)
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- Built-in useful extensions (available in all PostgreSQL distributions)
CREATE EXTENSION IF NOT EXISTS pgstattuple;      -- Tuple-level statistics
CREATE EXTENSION IF NOT EXISTS pg_buffercache;   -- Buffer cache inspection
CREATE EXTENSION IF NOT EXISTS pg_prewarm;       -- Prewarm buffer cache
CREATE EXTENSION IF NOT EXISTS pageinspect;      -- Low-level page inspection

-- Utility extensions
CREATE EXTENSION IF NOT EXISTS pgcrypto;         -- Cryptographic functions
CREATE EXTENSION IF NOT EXISTS citext;           -- Case-insensitive text type
CREATE EXTENSION IF NOT EXISTS btree_gist;       -- GiST index for exclusion constraints
CREATE EXTENSION IF NOT EXISTS pg_trgm;          -- Trigram matching for fuzzy search
