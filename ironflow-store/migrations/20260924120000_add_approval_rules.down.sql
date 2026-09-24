DROP TABLE iam.user_groups;

ALTER TABLE ironflow.steps
    DROP COLUMN approvals,
    DROP COLUMN approval_requirement;
