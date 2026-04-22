import { useLoaderData, Link, useNavigation } from "react-router";
import type { LoaderFunctionArgs } from "react-router";
import { useQueryStates, parseAsInteger } from "nuqs";
import type { RunResponse } from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { useRevalidateOnEvent } from "@/app/hooks/use-revalidate-on-event";
import { Pagination } from "@/app/components/Pagination";
import { RunFilters } from "./_components/RunFilters";
import { RunsTable } from "./_components/RunsTable";
import { Button } from "@/components/ui/button";
import { Plus } from "lucide-react";
import { useAppSelector } from "@/app/store";

const PER_PAGE = 20;

export interface RunsListLoaderData {
	runs: RunResponse[];
	meta: { page: number; per_page: number; total: number } | undefined;
}

export async function loader({ request }: LoaderFunctionArgs) {
	const url = new URL(request.url);
	const page = url.searchParams.get("page") ?? "1";
	const workflow = url.searchParams.get("workflow") ?? "";
	const status = url.searchParams.get("status") ?? "";
	const hasSteps = url.searchParams.get("has_steps") ?? "true";
	const label = url.searchParams.get("label") ?? "";

	const params = new URLSearchParams();
	params.set("page", page);
	params.set("per_page", String(PER_PAGE));
	if (workflow) params.set("workflow", workflow);
	if (status) params.set("status", status);
	if (hasSteps === "true") params.set("has_steps", "true");
	if (label) params.set("label", label);

	const res = await api.get<RunResponse[]>(`/runs?${params}`);
	return { runs: res.data, meta: res.meta };
}

export function Component() {
	const { runs, meta } = useLoaderData() as RunsListLoaderData;
	const navigation = useNavigation();
	const isLoading = navigation.state === "loading";
	const auth = useAppSelector((state) => state.auth);
	const isAdmin = auth.status === "authenticated" && auth.user.is_admin;
	useDocumentMeta({
		title: "Runs",
		description: "Monitor and manage workflow executions.",
	});

	useRevalidateOnEvent();

	const [queryFilters, setQueryFilters] = useQueryStates(
		{
			page: parseAsInteger.withDefault(1).withOptions({
				shallow: false,
			}),
		},
		{ history: "push" },
	);

	const currentPage = queryFilters.page;
	const totalPages = meta ? Math.ceil(meta.total / meta.per_page) : 1;

	const handlePageChange = (page: number) => {
		setQueryFilters({ page });
	};

	return (
		<HeaderApp
			title="Runs"
			description="Monitor and manage workflow executions."
			titleItem={
				isAdmin ? (
					<Link to="/workflows" className="block">
						<Button className="w-full sm:w-fit gap-1.5">
							<Plus className="w-4 h-4" />
							New Run
						</Button>
					</Link>
				) : undefined
			}
		>
			<div className="space-y-6">
				<RunFilters />
				<div
					className={
						isLoading ? "opacity-50 pointer-events-none transition-opacity" : ""
					}
				>
					<RunsTable runs={runs} />
				</div>
				<Pagination
					currentPage={currentPage}
					totalPages={totalPages}
					total={meta?.total ?? runs.length}
					onPageChange={handlePageChange}
				/>
			</div>
		</HeaderApp>
	);
}
