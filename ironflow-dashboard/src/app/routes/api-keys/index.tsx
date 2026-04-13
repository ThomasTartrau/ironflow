import { useState } from "react";
import { useLoaderData, useNavigate, useRevalidator } from "react-router";
import { Plus, Trash2 } from "lucide-react";
import { api } from "@/app/lib/api";
import type { ApiKeyResponse, ApiKeyScope } from "@/app/lib/types";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { withToast } from "@/app/lib/api-toast";
import { deleteApiKey } from "./_actions/actions";

export async function loader() {
  const res = await api.get<ApiKeyResponse[]>("/api-keys");
  return { apiKeys: res.data };
}

const SCOPE_LABELS: Record<ApiKeyScope, string> = {
  workflows_read: "Workflows Read",
  runs_read: "Runs Read",
  runs_write: "Runs Write",
  runs_manage: "Runs Manage",
  stats_read: "Stats Read",
};

function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString("fr-FR", {
    day: "2-digit",
    month: "short",
    year: "numeric",
  });
}

export function Component() {
  const { apiKeys } = useLoaderData() as { apiKeys: ApiKeyResponse[] };
  const navigate = useNavigate();
  const revalidator = useRevalidator();
  const [deletingId, setDeletingId] = useState<string | null>(null);

  useDocumentMeta({
    title: "API Keys",
    description: "Manage your API keys for programmatic access.",
  });

  function handleDelete(id: string, name: string) {
    setDeletingId(id);
    withToast(deleteApiKey(id), {
      loading: `Deleting ${name}...`,
      success: `${name} deleted`,
    })
      .then(() => revalidator.revalidate())
      .catch(() => {})
      .finally(() => setDeletingId(null));
  }

  return (
    <HeaderApp
      title="API Keys"
      description="Manage your API keys for programmatic access."
      titleItem={
        <Button onClick={() => navigate("/api-keys/new")}>
          <Plus className="h-4 w-4 mr-1" />
          New API Key
        </Button>
      }
    >
      <div className="space-y-6">
        {apiKeys.length === 0 ? (
          <div className="text-center py-16 text-muted-foreground border rounded-lg bg-muted/20">
            No API keys yet. Create one to get started.
          </div>
        ) : (
          <div className="rounded-lg border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Prefix</TableHead>
                  <TableHead>Scopes</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Last Used</TableHead>
                  <TableHead>Created</TableHead>
                  <TableHead className="w-12" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {apiKeys.map((key) => (
                  <TableRow key={key.id}>
                    <TableCell className="font-medium">{key.name}</TableCell>
                    <TableCell>
                      <code className="text-xs bg-muted px-1.5 py-0.5 rounded">
                        {key.key_prefix}...
                      </code>
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-wrap gap-1">
                        {key.scopes.map((scope) => (
                          <Badge
                            key={scope}
                            variant="outline"
                            className="text-xs"
                          >
                            {SCOPE_LABELS[scope] ?? scope}
                          </Badge>
                        ))}
                      </div>
                    </TableCell>
                    <TableCell>
                      {key.is_active ? (
                        <Badge
                          variant="default"
                          className="bg-green-500/10 text-green-700 border-green-500/20"
                        >
                          Active
                        </Badge>
                      ) : (
                        <Badge variant="secondary">Disabled</Badge>
                      )}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {key.last_used_at
                        ? formatDate(key.last_used_at)
                        : "Never"}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {formatDate(key.created_at)}
                    </TableCell>
                    <TableCell>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={() => handleDelete(key.id, key.name)}
                        disabled={deletingId === key.id}
                      >
                        <Trash2 className="h-4 w-4 text-destructive" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </div>
    </HeaderApp>
  );
}
