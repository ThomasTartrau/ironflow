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
