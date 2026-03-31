//! [`Page`] — paginated result set.

use serde::{Deserialize, Serialize};

/// A paginated result set.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::Page;
///
/// let page: Page<String> = Page {
///     items: vec!["a".to_string()],
///     total: 10,
///     page: 1,
///     per_page: 20,
/// };
/// assert_eq!(page.items.len(), 1);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    /// The items in this page.
    pub items: Vec<T>,
    /// Total number of items matching the filter.
    pub total: u64,
    /// Current page number (1-based).
    pub page: u32,
    /// Items per page.
    pub per_page: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let page = Page {
            items: vec![1, 2, 3],
            total: 100,
            page: 1,
            per_page: 20,
        };
        let json = serde_json::to_string(&page).expect("serialize");
        let back: Page<i32> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.items, vec![1, 2, 3]);
        assert_eq!(back.total, 100);
    }
}
