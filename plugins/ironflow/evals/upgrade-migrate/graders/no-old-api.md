---
type: regex
pattern: 'ApprovalRule::new\(|\.with_rule\(\s*ApprovalRule|\.allow_tool\("Read"\)\s*\.output|review\s*\.json(?:::<\w+>)?\(\)\?;'
match: not_contains
---
