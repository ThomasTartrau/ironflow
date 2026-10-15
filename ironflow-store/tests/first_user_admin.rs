//! The first account created on an empty store becomes admin, and only one
//! does, however many sign-ups race for it.
//!
//! The PostgreSQL test needs a real database:
//!
//! ```sh
//! scripts/test-postgres.sh -p ironflow-store --features store-postgres \
//!     --test first_user_admin -- --ignored
//! ```

use ironflow_store::entities::NewUser;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::user_store::UserStore;
use tokio::task::JoinSet;

/// Concurrent sign-ups racing for the first-admin decision.
const SIGN_UPS: usize = 20;

/// A sign-up that lets the store decide whether the account is admin.
fn sign_up(i: usize) -> NewUser {
    NewUser {
        email: format!("racer-{i}@example.com"),
        username: format!("racer-{i}"),
        password_hash: "hash".to_string(),
        is_admin: None,
    }
}

#[tokio::test]
async fn first_user_admin_is_unique_under_concurrent_sign_ups_in_memory() {
    let store = InMemoryStore::new();

    let mut sign_ups = JoinSet::new();
    for i in 0..SIGN_UPS {
        let store = store.clone();
        sign_ups.spawn(async move { store.create_user(sign_up(i)).await });
    }
    while let Some(created) = sign_ups.join_next().await {
        created.expect("task panicked").expect("create user");
    }

    let users = store.list_users(1, 100).await.expect("list users");
    assert_eq!(users.items.len(), SIGN_UPS);
    let admins = users.items.iter().filter(|u| u.is_admin).count();
    assert_eq!(admins, 1);
}

#[cfg(feature = "store-postgres")]
mod postgres {
    use std::env::var;
    use std::time::Duration;

    use ironflow_store::postgres::PostgresStore;
    use ironflow_store::user_store::UserStore;
    use sqlx::{Executor, PgPool};
    use tokio::task::JoinSet;
    use tokio::time::timeout;
    use uuid::Uuid;

    use super::{SIGN_UPS, sign_up};

    const TEST_TIMEOUT: Duration = Duration::from_secs(60);

    fn database_url() -> String {
        var("DATABASE_URL").expect("DATABASE_URL must be set")
    }

    /// `url` with its database replaced by `name`, query string kept.
    fn with_database(url: &str, name: &str) -> String {
        let authority = url.find("://").expect("DATABASE_URL has no scheme") + 3;
        let path = url[authority..]
            .find('/')
            .map_or(url.len(), |offset| authority + offset);
        let query = url[path..]
            .find('?')
            .map_or("", |offset| &url[path + offset..]);
        format!("{}/{name}{query}", &url[..path])
    }

    /// A database created empty for one test, so `iam.users` starts empty
    /// whatever the shared `DATABASE_URL` database holds.
    struct BlankDatabase {
        admin: PgPool,
        name: String,
        url: String,
    }

    impl BlankDatabase {
        async fn create() -> Self {
            let admin_url = database_url();
            let admin = PgPool::connect(&admin_url)
                .await
                .expect("failed to connect to PostgreSQL");
            let name = format!("ironflow_blank_{}", Uuid::now_v7().simple());
            admin
                .execute(format!(r#"CREATE DATABASE "{name}""#).as_str())
                .await
                .expect("failed to create the blank database");
            let url = with_database(&admin_url, &name);
            Self { admin, name, url }
        }

        async fn drop(self) {
            self.admin
                .execute(format!(r#"DROP DATABASE "{}" WITH (FORCE)"#, self.name).as_str())
                .await
                .expect("failed to drop the blank database");
        }
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn first_user_admin_is_unique_under_concurrent_sign_ups_on_postgres() {
        timeout(TEST_TIMEOUT, async {
            let db = BlankDatabase::create().await;
            let store = PostgresStore::new(&db.url)
                .await
                .expect("failed to connect to the blank database");

            let mut sign_ups = JoinSet::new();
            for i in 0..SIGN_UPS {
                let store = store.clone();
                sign_ups.spawn(async move { store.create_user(sign_up(i)).await });
            }
            let mut created = Vec::new();
            while let Some(result) = sign_ups.join_next().await {
                created.push(result.expect("task panicked"));
            }
            let users = store.list_users(1, 100).await;

            db.drop().await;

            for result in created {
                result.expect("create user");
            }
            let users = users.expect("list users");
            assert_eq!(users.items.len(), SIGN_UPS);
            let admins = users.items.iter().filter(|u| u.is_admin).count();
            assert_eq!(admins, 1);
        })
        .await
        .expect("test timed out");
    }
}
