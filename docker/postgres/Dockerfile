FROM timescale/timescaledb:latest-pg16

COPY postgresql.conf /etc/postgresql/postgresql.conf
COPY initdb/ /docker-entrypoint-initdb.d/

CMD ["postgres", "-c", "config_file=/etc/postgresql/postgresql.conf"]
