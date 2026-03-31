-- Create lib_fsm schema for SQL-side finite state machine support

CREATE SCHEMA IF NOT EXISTS lib_fsm;

-- ============================================================================
-- Abstract State Machine Definition Tables
-- ============================================================================

-- Abstract state machine definition
CREATE TABLE lib_fsm.abstract_state_machine (
    abstract_machine__id UUID PRIMARY KEY,
    name VARCHAR(30) NOT NULL UNIQUE,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Abstract states within a machine
CREATE TABLE lib_fsm.abstract_state (
    abstract_machine__id UUID NOT NULL REFERENCES lib_fsm.abstract_state_machine (abstract_machine__id) ON DELETE CASCADE,
    abstract_state__id UUID NOT NULL,
    name VARCHAR(50) NOT NULL,
    description TEXT,
    is_initial BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (abstract_machine__id, abstract_state__id),
    UNIQUE (abstract_state__id)
);

-- Abstract transitions between states
CREATE TABLE lib_fsm.abstract_transition (
    from_abstract_state__id UUID NOT NULL REFERENCES lib_fsm.abstract_state (abstract_state__id) ON DELETE CASCADE,
    to_abstract_state__id UUID NOT NULL REFERENCES lib_fsm.abstract_state (abstract_state__id) ON DELETE CASCADE,
    event VARCHAR(30) NOT NULL,
    description TEXT,
    PRIMARY KEY (from_abstract_state__id, event, to_abstract_state__id),
    UNIQUE (from_abstract_state__id, event)
);

CREATE INDEX idx_abstract_transition_from ON lib_fsm.abstract_transition (from_abstract_state__id);
CREATE INDEX idx_abstract_transition_to ON lib_fsm.abstract_transition (to_abstract_state__id);

-- ============================================================================
-- State Machine Instance Tables
-- ============================================================================

-- State machine instance (current state)
CREATE TABLE lib_fsm.state_machine (
    state_machine__id UUID PRIMARY KEY,
    abstract_state__id UUID NOT NULL REFERENCES lib_fsm.abstract_state (abstract_state__id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Event history for a state machine instance
CREATE TABLE lib_fsm.state_machine_event (
    event_id SERIAL PRIMARY KEY,
    state_machine__id UUID NOT NULL REFERENCES lib_fsm.state_machine (state_machine__id) ON DELETE CASCADE,
    abstract_state__id UUID NOT NULL REFERENCES lib_fsm.abstract_state (abstract_state__id) ON DELETE RESTRICT,
    event VARCHAR(30) NOT NULL,
    abstract_state_name VARCHAR(50) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_state_machine_event_sm ON lib_fsm.state_machine_event (state_machine__id);
CREATE INDEX idx_state_machine_event_created ON lib_fsm.state_machine_event (created_at DESC);

-- ============================================================================
-- Helper Functions
-- ============================================================================

-- Store event (internal helper)
CREATE OR REPLACE FUNCTION lib_fsm._state_machine_store_event(
    p_state_machine__id UUID,
    p_abstract_state__id UUID,
    p_event VARCHAR(30),
    p_abstract_state_name VARCHAR(50)
)
RETURNS VOID
LANGUAGE plpgsql
AS $$
BEGIN
    INSERT INTO lib_fsm.state_machine_event (
        state_machine__id,
        abstract_state__id,
        event,
        abstract_state_name,
        created_at
    ) VALUES (
        p_state_machine__id,
        p_abstract_state__id,
        p_event,
        p_abstract_state_name,
        NOW()
    );
END;
$$;

-- Create an abstract state machine
CREATE OR REPLACE FUNCTION lib_fsm.abstract_machine_create(
    p_name VARCHAR(30),
    p_description TEXT DEFAULT NULL
)
RETURNS UUID
LANGUAGE plpgsql
AS $$
DECLARE
    v_machine_id UUID;
BEGIN
    v_machine_id := gen_random_uuid();
    INSERT INTO lib_fsm.abstract_state_machine (
        abstract_machine__id,
        name,
        description,
        created_at
    ) VALUES (
        v_machine_id,
        p_name,
        p_description,
        NOW()
    );
    RETURN v_machine_id;
END;
$$;

-- Create an abstract state
CREATE OR REPLACE FUNCTION lib_fsm.abstract_state_create(
    p_abstract_machine__id UUID,
    p_name VARCHAR(50),
    p_description TEXT DEFAULT NULL,
    p_is_initial BOOLEAN DEFAULT FALSE
)
RETURNS UUID
LANGUAGE plpgsql
AS $$
DECLARE
    v_state_id UUID;
BEGIN
    v_state_id := gen_random_uuid();
    INSERT INTO lib_fsm.abstract_state (
        abstract_machine__id,
        abstract_state__id,
        name,
        description,
        is_initial
    ) VALUES (
        p_abstract_machine__id,
        v_state_id,
        p_name,
        p_description,
        p_is_initial
    );
    RETURN v_state_id;
END;
$$;

-- Create an abstract transition
CREATE OR REPLACE FUNCTION lib_fsm.abstract_transition_create(
    p_from_abstract_state__id UUID,
    p_event VARCHAR(30),
    p_to_abstract_state__id UUID,
    p_description TEXT DEFAULT NULL
)
RETURNS VOID
LANGUAGE plpgsql
AS $$
BEGIN
    INSERT INTO lib_fsm.abstract_transition (
        from_abstract_state__id,
        to_abstract_state__id,
        event,
        description
    ) VALUES (
        p_from_abstract_state__id,
        p_to_abstract_state__id,
        p_event,
        p_description
    );
END;
$$;

-- Create a state machine instance at initial state
CREATE OR REPLACE FUNCTION lib_fsm.state_machine_create(
    p_abstract_machine__id UUID
)
RETURNS UUID
LANGUAGE plpgsql
AS $$
DECLARE
    v_state_machine__id UUID;
    v_initial_state__id UUID;
BEGIN
    -- Find the initial state
    SELECT abstract_state__id INTO v_initial_state__id
    FROM lib_fsm.abstract_state
    WHERE abstract_machine__id = p_abstract_machine__id
      AND is_initial = TRUE
    LIMIT 1;

    IF v_initial_state__id IS NULL THEN
        RAISE EXCEPTION 'No initial state found for machine %', p_abstract_machine__id;
    END IF;

    -- Create the state machine instance
    v_state_machine__id := gen_random_uuid();
    INSERT INTO lib_fsm.state_machine (
        state_machine__id,
        abstract_state__id,
        created_at,
        updated_at
    ) VALUES (
        v_state_machine__id,
        v_initial_state__id,
        NOW(),
        NOW()
    );

    RETURN v_state_machine__id;
END;
$$;

-- Get current state of a state machine
CREATE OR REPLACE FUNCTION lib_fsm.state_machine_get(
    p_state_machine__id UUID
)
RETURNS TABLE (
    abstract_state__id UUID,
    name VARCHAR(50),
    description TEXT
)
LANGUAGE plpgsql
AS $$
BEGIN
    RETURN QUERY
    SELECT
        ast.abstract_state__id,
        ast.name,
        ast.description
    FROM lib_fsm.state_machine sm
    JOIN lib_fsm.abstract_state ast ON ast.abstract_state__id = sm.abstract_state__id
    WHERE sm.state_machine__id = p_state_machine__id;
END;
$$;

-- Perform a state transition
CREATE OR REPLACE FUNCTION lib_fsm.state_machine_transition(
    p_state_machine__id UUID,
    p_event VARCHAR(30)
)
RETURNS TABLE (
    abstract_state__id UUID,
    name VARCHAR(50),
    description TEXT,
    created_at TIMESTAMPTZ
)
LANGUAGE plpgsql
AS $$
DECLARE
    v_current_state__id UUID;
    v_next_state__id UUID;
    v_next_state_name VARCHAR(50);
    v_next_state_desc TEXT;
BEGIN
    -- Get current state
    SELECT sm.abstract_state__id INTO v_current_state__id
    FROM lib_fsm.state_machine sm
    WHERE sm.state_machine__id = p_state_machine__id
    FOR UPDATE;

    IF v_current_state__id IS NULL THEN
        RAISE EXCEPTION 'State machine % not found', p_state_machine__id;
    END IF;

    -- Find the transition
    SELECT at.to_abstract_state__id INTO v_next_state__id
    FROM lib_fsm.abstract_transition at
    WHERE at.from_abstract_state__id = v_current_state__id
      AND at.event = p_event
    LIMIT 1;

    IF v_next_state__id IS NULL THEN
        RAISE EXCEPTION 'Invalid transition from state % with event %', v_current_state__id, p_event;
    END IF;

    -- Get next state details
    SELECT ast.name, ast.description INTO v_next_state_name, v_next_state_desc
    FROM lib_fsm.abstract_state ast
    WHERE ast.abstract_state__id = v_next_state__id;

    -- Update state machine to next state
    UPDATE lib_fsm.state_machine
    SET abstract_state__id = v_next_state__id,
        updated_at = NOW()
    WHERE state_machine__id = p_state_machine__id;

    -- Store event in history
    PERFORM lib_fsm._state_machine_store_event(
        p_state_machine__id,
        v_next_state__id,
        p_event,
        v_next_state_name
    );

    RETURN QUERY SELECT v_next_state__id, v_next_state_name, v_next_state_desc, NOW();
END;
$$;
