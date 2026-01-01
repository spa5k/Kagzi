--
-- PostgreSQL database dump
--

\restrict a5TDMedwL27L0Iq6onoprgwq7JvKXKY7ZLzsbA0fA40EQpBnIGd5ztzFjG0eiJV

-- Dumped from database version 18.0
-- Dumped by pg_dump version 18.0

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET transaction_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

--
-- Name: kagzi; Type: SCHEMA; Schema: -; Owner: -
--

CREATE SCHEMA kagzi;


--
-- Name: pgcrypto; Type: EXTENSION; Schema: -; Owner: -
--

CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;


--
-- Name: EXTENSION pgcrypto; Type: COMMENT; Schema: -; Owner: -
--

COMMENT ON EXTENSION pgcrypto IS 'cryptographic functions';


SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: schedule_firings; Type: TABLE; Schema: kagzi; Owner: -
--

CREATE TABLE kagzi.schedule_firings (
    id bigint NOT NULL,
    schedule_id uuid NOT NULL,
    fire_at timestamp with time zone NOT NULL,
    run_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: schedule_firings_id_seq; Type: SEQUENCE; Schema: kagzi; Owner: -
--

CREATE SEQUENCE kagzi.schedule_firings_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: schedule_firings_id_seq; Type: SEQUENCE OWNED BY; Schema: kagzi; Owner: -
--

ALTER SEQUENCE kagzi.schedule_firings_id_seq OWNED BY kagzi.schedule_firings.id;


--
-- Name: schedules; Type: TABLE; Schema: kagzi; Owner: -
--

CREATE TABLE kagzi.schedules (
    schedule_id uuid NOT NULL,
    namespace_id text DEFAULT 'default'::text NOT NULL,
    task_queue text NOT NULL,
    workflow_type text NOT NULL,
    cron_expr text NOT NULL,
    input bytea DEFAULT '\x'::bytea NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    max_catchup integer DEFAULT 100 NOT NULL,
    next_fire_at timestamp with time zone NOT NULL,
    last_fired_at timestamp with time zone,
    version text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: step_runs; Type: TABLE; Schema: kagzi; Owner: -
--

CREATE TABLE kagzi.step_runs (
    attempt_id uuid NOT NULL,
    run_id uuid,
    step_id text NOT NULL,
    namespace_id text DEFAULT 'default'::text NOT NULL,
    status text NOT NULL,
    output bytea,
    error text,
    attempts integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now(),
    started_at timestamp with time zone,
    finished_at timestamp with time zone,
    retry_policy jsonb,
    attempt_number integer DEFAULT 1 NOT NULL,
    is_latest boolean DEFAULT true,
    input bytea,
    child_workflow_run_id uuid,
    step_kind text DEFAULT 'FUNCTION'::text NOT NULL
);


--
-- Name: workers; Type: TABLE; Schema: kagzi; Owner: -
--

CREATE TABLE kagzi.workers (
    worker_id uuid DEFAULT gen_random_uuid() NOT NULL,
    namespace_id text DEFAULT 'default'::text NOT NULL,
    task_queue text NOT NULL,
    hostname text,
    pid integer,
    version text,
    workflow_types text[] DEFAULT '{}'::text[] NOT NULL,
    max_concurrent integer DEFAULT 100 NOT NULL,
    status text DEFAULT 'ONLINE'::text NOT NULL,
    active_count integer DEFAULT 0 NOT NULL,
    total_completed bigint DEFAULT 0 NOT NULL,
    total_failed bigint DEFAULT 0 NOT NULL,
    registered_at timestamp with time zone DEFAULT now() NOT NULL,
    last_heartbeat_at timestamp with time zone DEFAULT now() NOT NULL,
    deregistered_at timestamp with time zone,
    labels jsonb DEFAULT '{}'::jsonb
);


--
-- Name: workflow_payloads; Type: TABLE; Schema: kagzi; Owner: -
--

CREATE TABLE kagzi.workflow_payloads (
    run_id uuid NOT NULL,
    input bytea NOT NULL,
    output bytea,
    created_at timestamp with time zone DEFAULT now()
);


--
-- Name: workflow_runs; Type: TABLE; Schema: kagzi; Owner: -
--

CREATE TABLE kagzi.workflow_runs (
    run_id uuid NOT NULL,
    namespace_id text DEFAULT 'default'::text NOT NULL,
    external_id text NOT NULL,
    task_queue text NOT NULL,
    workflow_type text NOT NULL,
    status text NOT NULL,
    locked_by text,
    attempts integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now(),
    started_at timestamp with time zone,
    finished_at timestamp with time zone,
    retry_policy jsonb,
    parent_step_attempt_id text,
    version text,
    error text,
    available_at timestamp with time zone,
    CONSTRAINT chk_available_at CHECK (((status = ANY (ARRAY['COMPLETED'::text, 'FAILED'::text, 'CANCELLED'::text])) OR (available_at IS NOT NULL)))
);


--
-- Name: _sqlx_migrations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public._sqlx_migrations (
    version bigint NOT NULL,
    description text NOT NULL,
    installed_on timestamp with time zone DEFAULT now() NOT NULL,
    success boolean NOT NULL,
    checksum bytea NOT NULL,
    execution_time bigint NOT NULL
);


--
-- Name: schedule_firings id; Type: DEFAULT; Schema: kagzi; Owner: -
--

ALTER TABLE ONLY kagzi.schedule_firings ALTER COLUMN id SET DEFAULT nextval('kagzi.schedule_firings_id_seq'::regclass);


--
-- Name: schedule_firings schedule_firings_pkey; Type: CONSTRAINT; Schema: kagzi; Owner: -
--

ALTER TABLE ONLY kagzi.schedule_firings
    ADD CONSTRAINT schedule_firings_pkey PRIMARY KEY (id);


--
-- Name: schedule_firings schedule_firings_schedule_id_fire_at_key; Type: CONSTRAINT; Schema: kagzi; Owner: -
--

ALTER TABLE ONLY kagzi.schedule_firings
    ADD CONSTRAINT schedule_firings_schedule_id_fire_at_key UNIQUE (schedule_id, fire_at);


--
-- Name: schedules schedules_pkey; Type: CONSTRAINT; Schema: kagzi; Owner: -
--

ALTER TABLE ONLY kagzi.schedules
    ADD CONSTRAINT schedules_pkey PRIMARY KEY (schedule_id);


--
-- Name: step_runs step_runs_pkey; Type: CONSTRAINT; Schema: kagzi; Owner: -
--

ALTER TABLE ONLY kagzi.step_runs
    ADD CONSTRAINT step_runs_pkey PRIMARY KEY (attempt_id);


--
-- Name: workers workers_pkey; Type: CONSTRAINT; Schema: kagzi; Owner: -
--

ALTER TABLE ONLY kagzi.workers
    ADD CONSTRAINT workers_pkey PRIMARY KEY (worker_id);


--
-- Name: workflow_payloads workflow_payloads_pkey; Type: CONSTRAINT; Schema: kagzi; Owner: -
--

ALTER TABLE ONLY kagzi.workflow_payloads
    ADD CONSTRAINT workflow_payloads_pkey PRIMARY KEY (run_id);


--
-- Name: workflow_runs workflow_runs_pkey; Type: CONSTRAINT; Schema: kagzi; Owner: -
--

ALTER TABLE ONLY kagzi.workflow_runs
    ADD CONSTRAINT workflow_runs_pkey PRIMARY KEY (run_id);


--
-- Name: _sqlx_migrations _sqlx_migrations_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public._sqlx_migrations
    ADD CONSTRAINT _sqlx_migrations_pkey PRIMARY KEY (version);


--
-- Name: idx_schedules_due; Type: INDEX; Schema: kagzi; Owner: -
--

CREATE INDEX idx_schedules_due ON kagzi.schedules USING btree (namespace_id, next_fire_at) WHERE (enabled = true);


--
-- Name: idx_schedules_ns_queue; Type: INDEX; Schema: kagzi; Owner: -
--

CREATE INDEX idx_schedules_ns_queue ON kagzi.schedules USING btree (namespace_id, task_queue);


--
-- Name: idx_step_runs_history; Type: INDEX; Schema: kagzi; Owner: -
--

CREATE INDEX idx_step_runs_history ON kagzi.step_runs USING btree (run_id, step_id, attempt_number);


--
-- Name: idx_step_runs_latest; Type: INDEX; Schema: kagzi; Owner: -
--

CREATE UNIQUE INDEX idx_step_runs_latest ON kagzi.step_runs USING btree (run_id, step_id) WHERE (is_latest = true);


--
-- Name: idx_workers_active_unique; Type: INDEX; Schema: kagzi; Owner: -
--

CREATE UNIQUE INDEX idx_workers_active_unique ON kagzi.workers USING btree (namespace_id, task_queue, hostname, pid) WHERE (status <> 'OFFLINE'::text);


--
-- Name: idx_workers_heartbeat; Type: INDEX; Schema: kagzi; Owner: -
--

CREATE INDEX idx_workers_heartbeat ON kagzi.workers USING btree (status, last_heartbeat_at) WHERE (status <> 'OFFLINE'::text);


--
-- Name: idx_workers_queue; Type: INDEX; Schema: kagzi; Owner: -
--

CREATE INDEX idx_workers_queue ON kagzi.workers USING btree (namespace_id, task_queue, status) WHERE (status = 'ONLINE'::text);


--
-- Name: idx_workflow_available; Type: INDEX; Schema: kagzi; Owner: -
--

CREATE INDEX idx_workflow_available ON kagzi.workflow_runs USING btree (namespace_id, task_queue, available_at) WHERE (status = ANY (ARRAY['PENDING'::text, 'SLEEPING'::text, 'RUNNING'::text]));


--
-- Name: idx_workflow_runs_running; Type: INDEX; Schema: kagzi; Owner: -
--

CREATE INDEX idx_workflow_runs_running ON kagzi.workflow_runs USING btree (task_queue, namespace_id, workflow_type) WHERE (status = 'RUNNING'::text);


--
-- Name: idx_workflow_status_lookup; Type: INDEX; Schema: kagzi; Owner: -
--

CREATE INDEX idx_workflow_status_lookup ON kagzi.workflow_runs USING btree (namespace_id, status, created_at DESC);


--
-- Name: uq_active_workflow; Type: INDEX; Schema: kagzi; Owner: -
--

CREATE UNIQUE INDEX uq_active_workflow ON kagzi.workflow_runs USING btree (namespace_id, external_id) WHERE (status <> ALL (ARRAY['COMPLETED'::text, 'FAILED'::text, 'CANCELLED'::text]));


--
-- Name: schedule_firings schedule_firings_schedule_id_fkey; Type: FK CONSTRAINT; Schema: kagzi; Owner: -
--

ALTER TABLE ONLY kagzi.schedule_firings
    ADD CONSTRAINT schedule_firings_schedule_id_fkey FOREIGN KEY (schedule_id) REFERENCES kagzi.schedules(schedule_id) ON DELETE CASCADE;


--
-- Name: step_runs step_runs_run_id_fkey; Type: FK CONSTRAINT; Schema: kagzi; Owner: -
--

ALTER TABLE ONLY kagzi.step_runs
    ADD CONSTRAINT step_runs_run_id_fkey FOREIGN KEY (run_id) REFERENCES kagzi.workflow_runs(run_id);


--
-- Name: workflow_payloads workflow_payloads_run_id_fkey; Type: FK CONSTRAINT; Schema: kagzi; Owner: -
--

ALTER TABLE ONLY kagzi.workflow_payloads
    ADD CONSTRAINT workflow_payloads_run_id_fkey FOREIGN KEY (run_id) REFERENCES kagzi.workflow_runs(run_id) ON DELETE CASCADE;


--
-- PostgreSQL database dump complete
--

\unrestrict a5TDMedwL27L0Iq6onoprgwq7JvKXKY7ZLzsbA0fA40EQpBnIGd5ztzFjG0eiJV

