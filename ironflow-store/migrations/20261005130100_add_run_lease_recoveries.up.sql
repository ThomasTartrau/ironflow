-- Number of recoveries after a lost worker lease, bounded by max_retries and
-- counted apart from retry_count so a recovered run keeps its attempt number.
ALTER TABLE ironflow.runs ADD COLUMN lease_recoveries INT NOT NULL DEFAULT 0;
