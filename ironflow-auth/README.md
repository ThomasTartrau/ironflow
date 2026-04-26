# ironflow-auth

Authentication library for **ironflow** -- JWT tokens, password hashing, and Axum extractors.

## Features

- **JWT** -- HS256 access + refresh tokens with configurable expiration
- **Password hashing** -- Argon2id (OWASP-recommended parameters)
- **Cookie management** -- HttpOnly secure cookies for token storage
- **Axum extractor** -- `AuthenticatedUser` extractor for route handlers

## Quick start

```rust,no_run
use ironflow_auth::jwt::{JwtConfig, AccessToken, RefreshToken};
use ironflow_auth::password;
use uuid::Uuid;

fn example() {
    // Hash a password
    let hash = password::hash("my-password").unwrap();
    assert!(password::verify("my-password", &hash).unwrap());

    // Issue tokens
    let config = JwtConfig::new("secret-key");
    let user_id = Uuid::now_v7();
    let access = AccessToken::issue(&config, user_id).unwrap();
    let refresh = RefreshToken::issue(&config, user_id).unwrap();
}
```

## License

MIT License - see [LICENSE](../LICENSE) for details.
