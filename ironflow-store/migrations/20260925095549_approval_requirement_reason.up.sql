-- Approval rules are replaced by approvers computed in the workflow handler.
-- A stored requirement keeps its count and groups; the matched rule condition
-- becomes the audit `reason`, and the rule index and evaluation trace go away.
-- Rows already in the new shape are left alone, so the update is idempotent.
UPDATE ironflow.steps
SET approval_requirement = (approval_requirement - 'rule_index' - 'condition' - 'evaluated')
    || jsonb_build_object('reason', approval_requirement -> 'condition')
WHERE approval_requirement IS NOT NULL
  AND approval_requirement ?| ARRAY['rule_index', 'condition', 'evaluated'];
