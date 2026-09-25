-- Dynamic approval matrix: the requirement evaluated when a gate opens and the
-- votes collected so far.
ALTER TABLE ironflow.steps
    ADD COLUMN approval_requirement JSONB,
    ADD COLUMN approvals JSONB NOT NULL DEFAULT '[]'::jsonb;

-- Group membership, used to restrict who may vote on a gate whose rule lists
-- `approver_groups`.
CREATE TABLE iam.user_groups (
    user_id    UUID NOT NULL REFERENCES iam.users(id) ON DELETE CASCADE,
    group_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, group_name)
);

CREATE INDEX idx_user_groups_group_name ON iam.user_groups (group_name);
