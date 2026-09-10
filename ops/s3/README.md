# ironflow-ops-s3

AWS S3 integration for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows, powered by the [`aws-sdk-s3`](https://crates.io/crates/aws-sdk-s3) crate.

Provides S3 operations (objects, buckets, presigned URLs, multipart uploads, ACLs, tagging) as tracked workflow steps.

## Usage

```toml
[dependencies]
ironflow-ops-s3 = "0.1"
```

### Build a client

```rust,ignore
use ironflow_ops_s3::S3Client;

// From a workflow step (reads aws_access_key_id, aws_secret_access_key, aws_region,
// and optionally aws_endpoint_url from the secret store)
let s3 = S3Client::from_context(&ctx).await?;
```

### Tracked operations

```rust,ignore
use ironflow_ops_s3::S3Client;
use ironflow_ops_s3::objects::PutObject;

let s3 = S3Client::from_context(&ctx).await?;

let put = PutObject::new(&s3, "my-bucket", "reports/daily.json")
    .body(b"{\"status\": \"ok\"}".to_vec());

let output = ctx.operation("upload-report", &put).await?;
```

## Available operations

| Module | Operations |
|--------|------------|
| `objects` | Get, put, delete, copy, head, list |
| `buckets` | Create, delete, head, list |
| `bucket_config` | Versioning, lifecycle, CORS, policy |
| `presigned` | Presigned get, presigned put |
| `multipart` | Create, upload part, complete, abort, list |
| `acl` | Get ACL, put ACL |
| `tagging` | Get tags, put tags, delete tags |

## Authentication

Register AWS credentials in your workflow's secret store:

```yaml
secrets:
  - name: aws_access_key_id
    env: AWS_ACCESS_KEY_ID
  - name: aws_secret_access_key
    env: AWS_SECRET_ACCESS_KEY
  - name: aws_region              # optional, defaults to "us-east-1"
    env: AWS_REGION
  - name: aws_endpoint_url        # optional, for S3-compatible services (MinIO, R2)
    env: AWS_ENDPOINT_URL
```
