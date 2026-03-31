-- Create IAM schema and users table.

CREATE SCHEMA IF NOT EXISTS iam;

CREATE TABLE iam.users (
    id              UUID        PRIMARY KEY,
    email           VARCHAR(255) NOT NULL UNIQUE,
    username        VARCHAR(255) NOT NULL UNIQUE,
    password_hash   TEXT        NOT NULL,
    is_admin        BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_users_email ON iam.users (email);
CREATE INDEX idx_users_username ON iam.users (username);
