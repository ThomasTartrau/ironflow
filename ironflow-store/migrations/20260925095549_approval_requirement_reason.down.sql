-- Restore the approval-rule shape. The evaluation trace cannot be rebuilt and
-- comes back empty; a requirement carrying a reason is reported as rule #0 with
-- the reason as its condition, so it stays visible to the previous code.
UPDATE ironflow.steps
SET approval_requirement = (approval_requirement - 'reason')
    || jsonb_build_object(
        'rule_index', CASE
            WHEN jsonb_typeof(approval_requirement -> 'reason') = 'string' THEN to_jsonb(0)
            ELSE 'null'::jsonb
        END,
        'condition', COALESCE(approval_requirement -> 'reason', 'null'::jsonb),
        'evaluated', '[]'::jsonb
    )
WHERE approval_requirement IS NOT NULL
  AND approval_requirement ? 'reason';
