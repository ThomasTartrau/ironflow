# Sandboxed Claude Code pods

Manifests for running `K8sEphemeralProvider::sandboxed` agents in their own
namespace. See the mdBook guide "Kubernetes sandbox" for the provider side.

## Apply order

```sh
kubectl apply -f namespace-rbac.yaml          # namespace + worker Role
kubectl apply -f networkpolicy-deny-all.yaml  # default deny, DNS only
kubectl apply -f cilium-egress-anthropic.yaml # FQDN egress (Cilium)
kubectl apply -f managed-settings.yaml        # managed-settings presets
kubectl -n ironflow-agents create secret generic claude-oauth \
  --from-literal=token="$(claude setup-token)"
```

Apply the network policies before the first agent pod starts: a pod created
before its policy runs unconfined until the CNI catches up.

## Why the worker does not create the policies

The Role in `namespace-rbac.yaml` grants pods, pod logs and ConfigMaps, never
`networkpolicies` or `ciliumnetworkpolicies`. The worker runs the agents; if it
could also write the policies, a compromised worker (or a prompt-injected
workflow driving it) could open the egress it is meant to be confined by. The
policies belong to whoever administers the cluster, applied once, out of band.

Egress profiles follow the same split: the provider only sets the
`ironflow.io/egress-profile` label, and a policy written by the administrator
decides what that label opens.

## Auth proxy variant

With `K8sEphemeralProvider::auth_proxy`, the agent pods never receive a Claude
credential: they get the proxy URL (`ANTHROPIC_BASE_URL`) and an opaque token
bound to one run and one step (`ANTHROPIC_AUTH_TOKEN`). The `ironflow-auth-proxy`
service swaps that token for the real credential and is the only component
that reaches `api.anthropic.com`.

```sh
kubectl apply -f namespace-rbac.yaml            # namespace + worker Role
kubectl apply -f networkpolicy-deny-all.yaml    # default deny, DNS only
kubectl apply -f auth-proxy.yaml                # ironflow-system + proxy
kubectl -n ironflow-system create secret generic ironflow-auth-proxy-admin \
  --from-literal=admin-key="$(openssl rand -hex 32)"
kubectl apply -f networkpolicy-auth-proxy.yaml  # agent pods -> proxy
kubectl apply -f cilium-egress-auth-proxy.yaml  # instead of cilium-egress-anthropic.yaml
kubectl apply -f managed-settings.yaml          # managed-settings presets
```

- Do not create the `claude-oauth` secret in `ironflow-agents`, and do not apply
  `cilium-egress-anthropic.yaml`: the agent pods must not reach the API directly.
- The worker needs `IRONFLOW_AUTH_PROXY_ADMIN_KEY` (the same key as the
  `ironflow-auth-proxy-admin` secret) and a credential to hand to the proxy:
  Provider Accounts, or `CLAUDE_CODE_OAUTH_TOKEN` (else `ANTHROPIC_API_KEY`) in
  its own environment.
- Any Claude credential set on the provider or the step for the pod
  (`oauth_token_from_secret`, `oauth_credentials`, ...) fails the step.
- Registry: in memory by default (one replica). Set
  `IRONFLOW_AUTH_PROXY_DATABASE_URL` and an encryption key
  (`IRONFLOW_SECRET_KEYS`) for a shared PostgreSQL registry: several replicas,
  tokens survive restarts, the proxy needs egress to PostgreSQL.
- Under Cilium, uncomment the 5432 rule in section (b) of
  `cilium-egress-auth-proxy.yaml` when the shared registry is on. With standard
  NetworkPolicies only, nothing restricts the proxy's egress unless you add a
  policy on the proxy pods; do not add 5432 to `networkpolicy-auth-proxy.yaml`,
  which selects the agent pods.

Check that an agent pod holds no secret while it runs:

```sh
kubectl -n ironflow-agents exec <agent pod> -- env | grep -c 'sk-ant'   # 0
```
