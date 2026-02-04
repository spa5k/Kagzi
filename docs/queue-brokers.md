# Broker Backends (NATS + Kafka)

Kagzi’s external brokers are used as a **work-signal bus** (wakeup distribution). They are never authoritative for execution.

Core rule:

- A broker delivery is **never a claim**. Workers must call `ClaimTask` and only execute if the DB lease/claim succeeds.

## Local setup

1. Start Postgres (existing compose):

```bash
just db-up
just migrate
```

2. Start broker(s) (separate compose; does not modify `docker-compose.yml`):

```bash
docker-compose -f docker/queue/docker-compose.nats.yml up -d
# or
docker-compose -f docker/queue/docker-compose.kafka.yml up -d
```

This starts (depending on which file you ran):

- NATS on `nats://localhost:4222` (JetStream enabled) and HTTP monitoring on `http://localhost:8222`
- Kafka (KRaft) on `localhost:9094` plus a one-shot init container that creates topic `kagzi-work` (16 partitions)

## Server configuration

Postgres (default):

- `KAGZI_QUEUE_BACKEND=postgres`
- Workers use gRPC `SubscribeWork` for wakeups

NATS:

- `KAGZI_QUEUE_BACKEND=nats`
- `KAGZI_QUEUE_NATS_URL=localhost:4222`
- `KAGZI_QUEUE_NATS_SUBJECT_PREFIX=kagzi.work`
- `KAGZI_QUEUE_NATS_QUEUE_GROUP=kagzi-workers`

Kafka:

- `KAGZI_QUEUE_BACKEND=kafka`
- `KAGZI_QUEUE_KAFKA_BROKERS=localhost:9094`
- `KAGZI_QUEUE_KAFKA_TOPIC=kagzi-work`
- `KAGZI_QUEUE_KAFKA_GROUP_ID_PREFIX=kagzi-workers`

Run the server:

```bash
just dev
```

## Worker behavior

- In `postgres` backend mode, `SubscribeWork` is supported.
- In `nats`/`kafka` backend modes, workers must **direct-subscribe to the broker** and call `ClaimTask` on wakeup.
  - The server will return `FailedPrecondition` for `SubscribeWork` in these modes to avoid multi-server routing traps.

## Quick local smoke

NATS:

```bash
docker-compose -f docker/queue/docker-compose.nats.yml up -d
KAGZI_QUEUE_BACKEND=nats just dev
just example 13_broker_smoke nats
```

Kafka:

```bash
docker-compose -f docker/queue/docker-compose.kafka.yml up -d
KAGZI_QUEUE_BACKEND=kafka just dev
just example 13_broker_smoke kafka
```
