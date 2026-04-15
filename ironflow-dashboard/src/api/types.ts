/* eslint-disable */
/* tslint:disable */
// @ts-nocheck
/*
 * ---------------------------------------------------------------
 * ## THIS FILE WAS GENERATED VIA SWAGGER-TYPESCRIPT-API        ##
 * ##                                                           ##
 * ## AUTHOR: acacode                                           ##
 * ## SOURCE: https://github.com/acacode/swagger-typescript-api ##
 * ---------------------------------------------------------------
 */

/**
 * Status of an individual step within a run.
 *
 * # Examples
 *
 * ```
 * use ironflow_store::entities::StepStatus;
 *
 * assert!(!StepStatus::Pending.is_terminal());
 * assert!(StepStatus::Completed.is_terminal());
 * ```
 */
export enum StepStatus {
  Pending = "pending",
  Running = "running",
  Completed = "completed",
  Failed = "failed",
  Skipped = "skipped",
  AwaitingApproval = "awaiting_approval",
  Rejected = "rejected",
}

/**
 * Status of a workflow run, forming a finite state machine.
 *
 * Valid transitions:
 * - `Pending` → `Running`, `Cancelled`
 * - `Running` → `Completed`, `Failed`, `Retrying`, `Cancelled`, `AwaitingApproval`
 * - `Retrying` → `Running`, `Failed`, `Cancelled`
 * - `AwaitingApproval` → `Running`, `Failed`, `Cancelled`
 *
 * Terminal states: `Completed`, `Failed`, `Cancelled`.
 *
 * # Examples
 *
 * ```
 * use ironflow_store::entities::RunStatus;
 *
 * assert!(RunStatus::Pending.can_transition_to(&RunStatus::Running));
 * assert!(!RunStatus::Pending.can_transition_to(&RunStatus::Completed));
 * assert!(!RunStatus::Completed.can_transition_to(&RunStatus::Running));
 * assert!(RunStatus::Running.can_transition_to(&RunStatus::AwaitingApproval));
 * assert!(RunStatus::AwaitingApproval.can_transition_to(&RunStatus::Running));
 * ```
 */
export enum RunStatus {
  Pending = "pending",
  Running = "running",
  Completed = "completed",
  Failed = "failed",
  Retrying = "retrying",
  Cancelled = "cancelled",
  AwaitingApproval = "awaiting_approval",
}

/**
 * Permission scope for an API key.
 *
 * Each scope grants access to a specific set of actions.
 * A key with no scopes has no permissions.
 */
export enum ApiKeyScope {
  WorkflowsRead = "workflows_read",
  RunsRead = "runs_read",
  RunsWrite = "runs_write",
  RunsManage = "runs_manage",
  StatsRead = "stats_read",
  Admin = "admin",
}

/** API key summary (never includes the hash or raw key). */
export interface ApiKeyResponse {
  /**
   * Creation date.
   * @format date-time
   */
  created_at: string;
  /**
   * Expiration date.
   * @format date-time
   */
  expires_at?: string | null;
  /**
   * API key ID.
   * @format uuid
   */
  id: string;
  /** Whether the key is active. */
  is_active: boolean;
  /** First characters for identification. */
  key_prefix: string;
  /**
   * Last used date.
   * @format date-time
   */
  last_used_at?: string | null;
  /** Key name. */
  name: string;
  /** Granted scopes. */
  scopes: ApiKeyScope[];
}

/** Request body for creating an API key. */
export interface CreateApiKeyRequest {
  /**
   * Optional expiration date (ISO 8601).
   * @format date-time
   */
  expires_at?: string | null;
  /** Human-readable name for this key. */
  name: string;
  /** Scopes to grant. */
  scopes: ApiKeyScope[];
}

/**
 * Response returned when creating an API key.
 * The raw key is only shown once.
 */
export interface CreateApiKeyResponse {
  /**
   * Creation date.
   * @format date-time
   */
  created_at: string;
  /**
   * Expiration date.
   * @format date-time
   */
  expires_at?: string | null;
  /**
   * API key ID.
   * @format uuid
   */
  id: string;
  /** The full raw API key (only returned at creation time). */
  key: string;
  /** First characters for identification. */
  key_prefix: string;
  /** Key name. */
  name: string;
  /** Granted scopes. */
  scopes: ApiKeyScope[];
}

/**
 * Request to trigger a workflow.
 *
 * # Examples
 *
 * ```
 * use ironflow_api::entities::CreateRunRequest;
 * use serde_json::json;
 *
 * let req = CreateRunRequest {
 *     workflow: "deploy".to_string(),
 *     payload: Some(json!({"env": "prod"})),
 * };
 * assert_eq!(req.workflow, "deploy");
 * ```
 */
export interface CreateRunRequest {
  /** Optional input payload for the workflow. */
  payload?: any;
  /** The workflow name to trigger. */
  workflow: string;
}

/** Request body for creating a user (admin only). */
export interface CreateUserRequest {
  /** Email address. */
  email: string;
  /** Whether the new user should be an admin. */
  is_admin: boolean;
  /** Plaintext password (min 8 characters). */
  password: string;
  /** Display username. */
  username: string;
}

/** Query parameters for listing runs. */
export interface ListRunsQuery {
  /**
   * When `true`, only return runs with at least one step.
   * When `false`, only return runs with no steps.
   */
  has_steps?: boolean | null;
  /**
   * Page number (1-based).
   * @format int32
   * @min 0
   */
  page?: number | null;
  /**
   * Items per page.
   * @format int32
   * @min 0
   */
  per_page?: number | null;
  /** Filter by run status. */
  status?: null | RunStatus;
  /** Filter by workflow name. */
  workflow?: string | null;
}

/** Query parameters for listing users. */
export interface ListUsersQuery {
  /**
   * Page number (1-based, defaults to 1).
   * @format int32
   * @min 0
   */
  page?: number | null;
  /**
   * Items per page (defaults to 20, max 100).
   * @format int32
   * @min 0
   */
  per_page?: number | null;
}

/** Query parameters for listing workflows. */
export interface ListWorkflowsQuery {
  /** Optional case-insensitive partial match on workflow name. */
  name?: string | null;
}

/** Current user profile response. */
export interface MeResponse {
  /** Email address. */
  email: string;
  /** Admin flag. */
  is_admin: boolean;
  /**
   * User ID.
   * @format uuid
   */
  user_id: string;
  /** Display username. */
  username: string;
}

/** Run detail response — includes steps. */
export interface RunDetailResponse {
  /** The run. */
  run: RunResponse;
  /** Associated steps, ordered by position. */
  steps: StepResponse[];
}

/**
 * Run response DTO — public API representation of a run.
 *
 * Maps from the internal [`Run`] model, exposing only necessary fields.
 *
 * # Examples
 *
 * ```
 * use ironflow_store::models::{Run, RunStatus, TriggerKind};
 * use ironflow_api::entities::RunResponse;
 * ```
 */
export interface RunResponse {
  /**
   * When execution completed.
   * @format date-time
   */
  completed_at?: string | null;
  /** Aggregated cost in USD. */
  cost_usd: string;
  /**
   * When created.
   * @format date-time
   */
  created_at: string;
  /**
   * Total duration in milliseconds.
   * @format int64
   * @min 0
   */
  duration_ms: number;
  /** Optional error message. */
  error?: string | null;
  /**
   * Unique run identifier.
   * @format uuid
   */
  id: string;
  /**
   * Maximum allowed retries.
   * @format int32
   * @min 0
   */
  max_retries: number;
  /**
   * Number of times retried.
   * @format int32
   * @min 0
   */
  retry_count: number;
  /**
   * When execution started.
   * @format date-time
   */
  started_at?: string | null;
  /** Current status. */
  status: RunStatus;
  /** How the run was triggered. */
  trigger: TriggerKind;
  /**
   * When last updated.
   * @format date-time
   */
  updated_at: string;
  /** Workflow name. */
  workflow_name: string;
}

/** A scope entry with its machine name and human-readable label. */
export interface ScopeEntry {
  /** Short description. */
  description: string;
  /** Human-readable label (e.g. "Runs Read"). */
  label: string;
  /** Machine-readable scope value (e.g. "runs_read"). */
  value: string;
}

/** Sign-in request body. */
export interface SignInRequest {
  /** Email address. */
  email: string;
  /** Plaintext password. */
  password: string;
}

/** Sign-up request body. */
export interface SignUpRequest {
  /** Email address. */
  email: string;
  /** Plaintext password (min 8 characters). */
  password: string;
  /** Display username. */
  username: string;
}

/**
 * Aggregate statistics response.
 *
 * Computed from all runs in the store.
 *
 * # Examples
 *
 * ```
 * use ironflow_api::entities::StatsResponse;
 * ```
 */
export interface StatsResponse {
  /**
   * Number of pending or running runs.
   * @format int64
   * @min 0
   */
  active_runs: number;
  /**
   * Number of cancelled runs.
   * @format int64
   * @min 0
   */
  cancelled_runs: number;
  /**
   * Number of completed runs.
   * @format int64
   * @min 0
   */
  completed_runs: number;
  /**
   * Number of failed runs.
   * @format int64
   * @min 0
   */
  failed_runs: number;
  /**
   * Success rate: completed / (completed + failed), as a percentage.
   * @format double
   */
  success_rate_percent: number;
  /** Aggregated cost across all runs in USD. */
  total_cost_usd: string;
  /**
   * Aggregated duration across all runs in milliseconds.
   * @format int64
   * @min 0
   */
  total_duration_ms: number;
  /**
   * Total number of runs.
   * @format int64
   * @min 0
   */
  total_runs: number;
}

/**
 * Step response DTO — public API representation of a step.
 *
 * # Examples
 *
 * ```
 * use ironflow_store::models::Step;
 * use ironflow_api::entities::StepResponse;
 * ```
 */
export interface StepResponse {
  /**
   * When execution completed.
   * @format date-time
   */
  completed_at?: string | null;
  /** Cost in USD. */
  cost_usd: string;
  /**
   * When created.
   * @format date-time
   */
  created_at: string;
  /** IDs of steps this step depends on (direct dependencies). */
  dependencies: string[];
  /**
   * Execution duration in milliseconds.
   * @format int64
   * @min 0
   */
  duration_ms: number;
  /** Optional error message. */
  error?: string | null;
  /**
   * Unique step identifier.
   * @format uuid
   */
  id: string;
  /** Input configuration. */
  input?: any;
  /**
   * Input token count (agent steps).
   * @format int64
   * @min 0
   */
  input_tokens?: number | null;
  /** Step operation type. */
  kind: string;
  /** Step name. */
  name: string;
  /** Step output. */
  output?: any;
  /**
   * Output token count (agent steps).
   * @format int64
   * @min 0
   */
  output_tokens?: number | null;
  /**
   * Execution order (0-based).
   * @format int32
   * @min 0
   */
  position: number;
  /**
   * Parent run ID.
   * @format uuid
   */
  run_id: string;
  /**
   * When execution started.
   * @format date-time
   */
  started_at?: string | null;
  /** Current status. */
  status: StepStatus;
  /**
   * When updated.
   * @format date-time
   */
  updated_at: string;
}

/** Sub-workflow detail included in the workflow response. */
export interface SubWorkflowDetail {
  /** Human-readable description. */
  description: string;
  /** Sub-workflow name. */
  name: string;
  /** Optional Rust source code of the handler. */
  source_code?: string | null;
}

/**
 * How a run was triggered.
 *
 * # Examples
 *
 * ```
 * use ironflow_store::entities::TriggerKind;
 *
 * let trigger = TriggerKind::Manual;
 * let json = serde_json::to_string(&trigger).unwrap();
 * assert!(json.contains("manual"));
 * ```
 */
export type TriggerKind =
  | {
      kind: "manual";
    }
  | {
      kind: "webhook";
      /** The webhook path that received the request. */
      path: string;
    }
  | {
      kind: "cron";
      /** The cron expression that fired. */
      schedule: string;
    }
  | {
      kind: "api";
    }
  | {
      kind: "retry";
      /**
       * The original run that failed.
       * @format uuid
       */
      parent_run_id: string;
    }
  | {
      kind: "workflow";
    };

/** Request body for updating a user's role (admin only). */
export interface UpdateRoleRequest {
  /** New admin status. */
  is_admin: boolean;
}

/** Response DTO for a user (never exposes password hash). */
export interface UserResponse {
  /**
   * Creation timestamp.
   * @format date-time
   */
  created_at: string;
  /** Email address. */
  email: string;
  /**
   * User ID.
   * @format uuid
   */
  id: string;
  /** Whether the user is an admin. */
  is_admin: boolean;
  /**
   * Last update timestamp.
   * @format date-time
   */
  updated_at: string;
  /** Display username. */
  username: string;
}

/** Workflow detail response. */
export interface WorkflowDetailResponse {
  /** Human-readable description. */
  description: string;
  /** Workflow name. */
  name: string;
  /** Optional Rust source code of the handler. */
  source_code?: string | null;
  /** Sub-workflows invoked by this handler (recursive, depth-limited). */
  sub_workflows: SubWorkflowDetail[];
}

export type QueryParamsType = Record<string | number, any>;
export type ResponseFormat = keyof Omit<Body, "body" | "bodyUsed">;

export interface FullRequestParams extends Omit<RequestInit, "body"> {
  /** set parameter to `true` for call `securityWorker` for this request */
  secure?: boolean;
  /** request path */
  path: string;
  /** content type of request body */
  type?: ContentType;
  /** query params */
  query?: QueryParamsType;
  /** format of response (i.e. response.json() -> format: "json") */
  format?: ResponseFormat;
  /** request body */
  body?: unknown;
  /** base url */
  baseUrl?: string;
  /** request cancellation token */
  cancelToken?: CancelToken;
}

export type RequestParams = Omit<
  FullRequestParams,
  "body" | "method" | "query" | "path"
>;

export interface ApiConfig<SecurityDataType = unknown> {
  baseUrl?: string;
  baseApiParams?: Omit<RequestParams, "baseUrl" | "cancelToken" | "signal">;
  securityWorker?: (
    securityData: SecurityDataType | null,
  ) => Promise<RequestParams | void> | RequestParams | void;
  customFetch?: typeof fetch;
}

export interface HttpResponse<D extends unknown, E extends unknown = unknown>
  extends Response {
  data: D;
  error: E;
}

type CancelToken = Symbol | string | number;

export enum ContentType {
  Json = "application/json",
  JsonApi = "application/vnd.api+json",
  FormData = "multipart/form-data",
  UrlEncoded = "application/x-www-form-urlencoded",
  Text = "text/plain",
}

export class HttpClient<SecurityDataType = unknown> {
  public baseUrl: string = "";
  private securityData: SecurityDataType | null = null;
  private securityWorker?: ApiConfig<SecurityDataType>["securityWorker"];
  private abortControllers = new Map<CancelToken, AbortController>();
  private customFetch = (...fetchParams: Parameters<typeof fetch>) =>
    fetch(...fetchParams);

  private baseApiParams: RequestParams = {
    credentials: "same-origin",
    headers: {},
    redirect: "follow",
    referrerPolicy: "no-referrer",
  };

  constructor(apiConfig: ApiConfig<SecurityDataType> = {}) {
    Object.assign(this, apiConfig);
  }

  public setSecurityData = (data: SecurityDataType | null) => {
    this.securityData = data;
  };

  protected encodeQueryParam(key: string, value: any) {
    const encodedKey = encodeURIComponent(key);
    return `${encodedKey}=${encodeURIComponent(typeof value === "number" ? value : `${value}`)}`;
  }

  protected addQueryParam(query: QueryParamsType, key: string) {
    return this.encodeQueryParam(key, query[key]);
  }

  protected addArrayQueryParam(query: QueryParamsType, key: string) {
    const value = query[key];
    return value.map((v: any) => this.encodeQueryParam(key, v)).join("&");
  }

  protected toQueryString(rawQuery?: QueryParamsType): string {
    const query = rawQuery || {};
    const keys = Object.keys(query).filter(
      (key) => "undefined" !== typeof query[key],
    );
    return keys
      .map((key) =>
        Array.isArray(query[key])
          ? this.addArrayQueryParam(query, key)
          : this.addQueryParam(query, key),
      )
      .join("&");
  }

  protected addQueryParams(rawQuery?: QueryParamsType): string {
    const queryString = this.toQueryString(rawQuery);
    return queryString ? `?${queryString}` : "";
  }

  private contentFormatters: Record<ContentType, (input: any) => any> = {
    [ContentType.Json]: (input: any) =>
      input !== null && (typeof input === "object" || typeof input === "string")
        ? JSON.stringify(input)
        : input,
    [ContentType.JsonApi]: (input: any) =>
      input !== null && (typeof input === "object" || typeof input === "string")
        ? JSON.stringify(input)
        : input,
    [ContentType.Text]: (input: any) =>
      input !== null && typeof input !== "string"
        ? JSON.stringify(input)
        : input,
    [ContentType.FormData]: (input: any) => {
      if (input instanceof FormData) {
        return input;
      }

      return Object.keys(input || {}).reduce((formData, key) => {
        const property = input[key];
        formData.append(
          key,
          property instanceof Blob
            ? property
            : typeof property === "object" && property !== null
              ? JSON.stringify(property)
              : `${property}`,
        );
        return formData;
      }, new FormData());
    },
    [ContentType.UrlEncoded]: (input: any) => this.toQueryString(input),
  };

  protected mergeRequestParams(
    params1: RequestParams,
    params2?: RequestParams,
  ): RequestParams {
    return {
      ...this.baseApiParams,
      ...params1,
      ...(params2 || {}),
      headers: {
        ...(this.baseApiParams.headers || {}),
        ...(params1.headers || {}),
        ...((params2 && params2.headers) || {}),
      },
    };
  }

  protected createAbortSignal = (
    cancelToken: CancelToken,
  ): AbortSignal | undefined => {
    if (this.abortControllers.has(cancelToken)) {
      const abortController = this.abortControllers.get(cancelToken);
      if (abortController) {
        return abortController.signal;
      }
      return void 0;
    }

    const abortController = new AbortController();
    this.abortControllers.set(cancelToken, abortController);
    return abortController.signal;
  };

  public abortRequest = (cancelToken: CancelToken) => {
    const abortController = this.abortControllers.get(cancelToken);

    if (abortController) {
      abortController.abort();
      this.abortControllers.delete(cancelToken);
    }
  };

  public request = async <T = any, E = any>({
    body,
    secure,
    path,
    type,
    query,
    format,
    baseUrl,
    cancelToken,
    ...params
  }: FullRequestParams): Promise<HttpResponse<T, E>> => {
    const secureParams =
      ((typeof secure === "boolean" ? secure : this.baseApiParams.secure) &&
        this.securityWorker &&
        (await this.securityWorker(this.securityData))) ||
      {};
    const requestParams = this.mergeRequestParams(params, secureParams);
    const queryString = query && this.toQueryString(query);
    const payloadFormatter = this.contentFormatters[type || ContentType.Json];
    const responseFormat = format || requestParams.format;

    return this.customFetch(
      `${baseUrl || this.baseUrl || ""}${path}${queryString ? `?${queryString}` : ""}`,
      {
        ...requestParams,
        headers: {
          ...(requestParams.headers || {}),
          ...(type && type !== ContentType.FormData
            ? { "Content-Type": type }
            : {}),
        },
        signal:
          (cancelToken
            ? this.createAbortSignal(cancelToken)
            : requestParams.signal) || null,
        body:
          typeof body === "undefined" || body === null
            ? null
            : payloadFormatter(body),
      },
    ).then(async (response) => {
      const r = response as HttpResponse<T, E>;
      r.data = null as unknown as T;
      r.error = null as unknown as E;

      const responseToParse = responseFormat ? response.clone() : response;
      const data = !responseFormat
        ? r
        : await responseToParse[responseFormat]()
            .then((data) => {
              if (r.ok) {
                r.data = data;
              } else {
                r.error = data;
              }
              return r;
            })
            .catch((e) => {
              r.error = e;
              return r;
            });

      if (cancelToken) {
        this.abortControllers.delete(cancelToken);
      }

      if (!response.ok) throw data;
      return data;
    });
  };
}

/**
 * @title Ironflow REST API
 * @version 1.0.0
 * @license MIT
 * @contact Thomas Tartrau
 *
 * REST API for the ironflow workflow engine
 */
export class Api<
  SecurityDataType extends unknown,
> extends HttpClient<SecurityDataType> {
  api = {
    /**
     * No description
     *
     * @tags api-keys
     * @name ListApiKeys
     * @summary List all API keys for the authenticated user.
     * @request GET:/api/v1/api-keys
     * @secure
     */
    listApiKeys: (params: RequestParams = {}) =>
      this.request<ApiKeyResponse[], void>({
        path: `/api/v1/api-keys`,
        method: "GET",
        secure: true,
        format: "json",
        ...params,
      }),

    /**
     * @description # Errors - 400 if the name is empty or scopes are invalid
     *
     * @tags api-keys
     * @name CreateApiKey
     * @summary Create a new API key for the authenticated user.
     * @request POST:/api/v1/api-keys
     * @secure
     */
    createApiKey: (data: CreateApiKeyRequest, params: RequestParams = {}) =>
      this.request<CreateApiKeyResponse, void>({
        path: `/api/v1/api-keys`,
        method: "POST",
        body: data,
        secure: true,
        type: ContentType.Json,
        format: "json",
        ...params,
      }),

    /**
     * @description Admins get all scopes, members only get read-only scopes.
     *
     * @tags api-keys
     * @name AvailableScopes
     * @summary Return the list of scopes the current user is allowed to assign to API keys.
     * @request GET:/api/v1/api-keys/scopes
     * @secure
     */
    availableScopes: (params: RequestParams = {}) =>
      this.request<ScopeEntry[], void>({
        path: `/api/v1/api-keys/scopes`,
        method: "GET",
        secure: true,
        format: "json",
        ...params,
      }),

    /**
     * @description # Errors - 404 if the key does not exist or belongs to another user
     *
     * @tags api-keys
     * @name DeleteApiKey
     * @summary Delete an API key owned by the authenticated user.
     * @request DELETE:/api/v1/api-keys/{id}
     * @secure
     */
    deleteApiKey: (id: string, params: RequestParams = {}) =>
      this.request<void, void>({
        path: `/api/v1/api-keys/${id}`,
        method: "DELETE",
        secure: true,
        ...params,
      }),

    /**
     * @description # Errors - 401 if no valid token is provided - 404 if the user no longer exists in the store
     *
     * @tags auth
     * @name Me
     * @summary Return the current user's profile.
     * @request GET:/api/v1/auth/me
     * @secure
     */
    me: (params: RequestParams = {}) =>
      this.request<MeResponse, void>({
        path: `/api/v1/auth/me`,
        method: "GET",
        secure: true,
        format: "json",
        ...params,
      }),

    /**
     * @description # Errors - 401 if the refresh token is missing, invalid, or expired
     *
     * @tags auth
     * @name Refresh
     * @summary Refresh the access token using a valid refresh token from cookies.
     * @request POST:/api/v1/auth/refresh
     */
    refresh: (params: RequestParams = {}) =>
      this.request<void, void>({
        path: `/api/v1/auth/refresh`,
        method: "POST",
        ...params,
      }),

    /**
     * @description Returns access and refresh tokens on success, and sets HttpOnly cookies. # Errors - 401 if email not found or password is wrong
     *
     * @tags auth
     * @name SignIn
     * @summary Authenticate a user with email and password.
     * @request POST:/api/v1/auth/sign-in
     */
    signIn: (data: SignInRequest, params: RequestParams = {}) =>
      this.request<void, void>({
        path: `/api/v1/auth/sign-in`,
        method: "POST",
        body: data,
        type: ContentType.Json,
        ...params,
      }),

    /**
     * No description
     *
     * @tags auth
     * @name SignOut
     * @summary Sign out the current user by clearing auth cookies.
     * @request POST:/api/v1/auth/sign-out
     * @secure
     */
    signOut: (params: RequestParams = {}) =>
      this.request<void, void>({
        path: `/api/v1/auth/sign-out`,
        method: "POST",
        secure: true,
        ...params,
      }),

    /**
     * @description Returns access and refresh tokens on success, and sets HttpOnly cookies. # Errors - 400 if email/username/password is invalid - 409 if email or username is already taken
     *
     * @tags auth
     * @name SignUp
     * @summary Register a new user with email and password.
     * @request POST:/api/v1/auth/sign-up
     */
    signUp: (data: SignUpRequest, params: RequestParams = {}) =>
      this.request<void, void>({
        path: `/api/v1/auth/sign-up`,
        method: "POST",
        body: data,
        type: ContentType.Json,
        ...params,
      }),

    /**
     * No description
     *
     * @tags health
     * @name HealthCheck
     * @summary Health check handler. Always returns 200 OK.
     * @request GET:/api/v1/health-check
     */
    healthCheck: (params: RequestParams = {}) =>
      this.request<void, any>({
        path: `/api/v1/health-check`,
        method: "GET",
        ...params,
      }),

    /**
     * @description # Query Parameters - `workflow` — Filter by workflow name (optional) - `status` — Filter by run status (optional) - `page` — Page number, 1-based (default: 1) - `per_page` — Items per page (default: 20, max: 100)
     *
     * @tags runs
     * @name ListRuns
     * @summary List runs with optional filtering and pagination.
     * @request GET:/api/v1/runs
     * @secure
     */
    listRuns: (
      query?: {
        /** Filter by workflow name. */
        workflow?: string | null;
        /** Filter by run status. */
        status?: null | RunStatus;
        /**
         * When `true`, only return runs with at least one step.
         * When `false`, only return runs with no steps.
         */
        has_steps?: boolean | null;
        /**
         * Page number (1-based).
         * @format int32
         * @min 0
         */
        page?: number | null;
        /**
         * Items per page.
         * @format int32
         * @min 0
         */
        per_page?: number | null;
      },
      params: RequestParams = {},
    ) =>
      this.request<void, void>({
        path: `/api/v1/runs`,
        method: "GET",
        query: query,
        secure: true,
        ...params,
      }),

    /**
     * @description Returns 201 Created with the newly enqueued run. Returns 400 Bad Request if the workflow is unknown.
     *
     * @tags runs
     * @name CreateRun
     * @summary Trigger a workflow by name.
     * @request POST:/api/v1/runs
     * @secure
     */
    createRun: (data: CreateRunRequest, params: RequestParams = {}) =>
      this.request<RunResponse, void>({
        path: `/api/v1/runs`,
        method: "POST",
        body: data,
        secure: true,
        type: ContentType.Json,
        format: "json",
        ...params,
      }),

    /**
     * @description Returns 404 if the run does not exist.
     *
     * @tags runs
     * @name GetRun
     * @summary Get a run by ID, including all its steps and dependency edges.
     * @request GET:/api/v1/runs/{id}
     * @secure
     */
    getRun: (id: string, params: RequestParams = {}) =>
      this.request<RunDetailResponse, void>({
        path: `/api/v1/runs/${id}`,
        method: "GET",
        secure: true,
        format: "json",
        ...params,
      }),

    /**
     * @description Transitions the run from `AwaitingApproval` back to `Running`. Returns 400 if the run is not in `AwaitingApproval` state.
     *
     * @tags runs
     * @name ApproveRun
     * @summary Approve a run that is awaiting human approval.
     * @request POST:/api/v1/runs/{id}/approve
     * @secure
     */
    approveRun: (id: string, params: RequestParams = {}) =>
      this.request<RunResponse, void>({
        path: `/api/v1/runs/${id}/approve`,
        method: "POST",
        secure: true,
        format: "json",
        ...params,
      }),

    /**
     * @description Transitions the run to `Cancelled` status. Returns 400 if the run is already in a terminal state.
     *
     * @tags runs
     * @name CancelRun
     * @summary Cancel a pending or running run.
     * @request POST:/api/v1/runs/{id}/cancel
     * @secure
     */
    cancelRun: (id: string, params: RequestParams = {}) =>
      this.request<RunResponse, void>({
        path: `/api/v1/runs/${id}/cancel`,
        method: "POST",
        secure: true,
        format: "json",
        ...params,
      }),

    /**
     * @description Transitions the run from `AwaitingApproval` to `Failed`. Returns 400 if the run is not in `AwaitingApproval` state.
     *
     * @tags runs
     * @name RejectRun
     * @summary Reject a run that is awaiting human approval.
     * @request POST:/api/v1/runs/{id}/reject
     * @secure
     */
    rejectRun: (id: string, params: RequestParams = {}) =>
      this.request<RunResponse, void>({
        path: `/api/v1/runs/${id}/reject`,
        method: "POST",
        secure: true,
        format: "json",
        ...params,
      }),

    /**
     * @description Creates a new `Pending` run with `TriggerKind::Retry` pointing to the original. Returns 400 if the run is not in a retryable state.
     *
     * @tags runs
     * @name RetryRun
     * @summary Retry a failed run.
     * @request POST:/api/v1/runs/{id}/retry
     * @secure
     */
    retryRun: (id: string, params: RequestParams = {}) =>
      this.request<RunResponse, void>({
        path: `/api/v1/runs/${id}/retry`,
        method: "POST",
        secure: true,
        format: "json",
        ...params,
      }),

    /**
     * No description
     *
     * @tags stats
     * @name GetStats
     * @summary Get aggregate statistics across all runs.
     * @request GET:/api/v1/stats
     * @secure
     */
    getStats: (params: RequestParams = {}) =>
      this.request<StatsResponse, void>({
        path: `/api/v1/stats`,
        method: "GET",
        secure: true,
        format: "json",
        ...params,
      }),

    /**
     * @description # Errors - 403 if the caller is not an admin
     *
     * @tags users
     * @name ListUsers
     * @summary List all users with pagination. Admin only.
     * @request GET:/api/v1/users
     * @secure
     */
    listUsers: (
      query?: {
        /**
         * Page number (1-based, defaults to 1).
         * @format int32
         * @min 0
         */
        page?: number | null;
        /**
         * Items per page (defaults to 20, max 100).
         * @format int32
         * @min 0
         */
        per_page?: number | null;
      },
      params: RequestParams = {},
    ) =>
      this.request<UserResponse[], void>({
        path: `/api/v1/users`,
        method: "GET",
        query: query,
        secure: true,
        format: "json",
        ...params,
      }),

    /**
     * @description # Errors - 403 if the caller is not an admin - 400 if input validation fails - 409 if email or username is already taken
     *
     * @tags users
     * @name CreateUser
     * @summary Create a new user account. Admin only.
     * @request POST:/api/v1/users
     * @secure
     */
    createUser: (data: CreateUserRequest, params: RequestParams = {}) =>
      this.request<UserResponse, void>({
        path: `/api/v1/users`,
        method: "POST",
        body: data,
        secure: true,
        type: ContentType.Json,
        format: "json",
        ...params,
      }),

    /**
     * @description # Errors - 403 if the caller is not an admin - 400 if trying to delete self - 404 if the user does not exist
     *
     * @tags users
     * @name DeleteUser
     * @summary Delete a user account. Admin only.
     * @request DELETE:/api/v1/users/{id}
     * @secure
     */
    deleteUser: (id: string, params: RequestParams = {}) =>
      this.request<void, void>({
        path: `/api/v1/users/${id}`,
        method: "DELETE",
        secure: true,
        ...params,
      }),

    /**
     * @description # Errors - 403 if the caller is not an admin - 400 if trying to change own role - 404 if the user does not exist
     *
     * @tags users
     * @name UpdateRole
     * @summary Update a user's admin role. Admin only.
     * @request PATCH:/api/v1/users/{id}/role
     * @secure
     */
    updateRole: (
      id: string,
      data: UpdateRoleRequest,
      params: RequestParams = {},
    ) =>
      this.request<UserResponse, void>({
        path: `/api/v1/users/${id}/role`,
        method: "PATCH",
        body: data,
        secure: true,
        type: ContentType.Json,
        format: "json",
        ...params,
      }),

    /**
     * @description # Query Parameters - `name` — Filter by workflow name, case-insensitive partial match (optional)
     *
     * @tags workflows
     * @name ListWorkflows
     * @summary List registered workflow names, optionally filtered by name.
     * @request GET:/api/v1/workflows
     * @secure
     */
    listWorkflows: (
      query?: {
        /** Optional case-insensitive partial match on workflow name. */
        name?: string | null;
      },
      params: RequestParams = {},
    ) =>
      this.request<void, void>({
        path: `/api/v1/workflows`,
        method: "GET",
        query: query,
        secure: true,
        ...params,
      }),

    /**
     * @description # Errors - 404 if the workflow is not registered
     *
     * @tags workflows
     * @name GetWorkflow
     * @summary Get details about a registered workflow.
     * @request GET:/api/v1/workflows/{name}
     * @secure
     */
    getWorkflow: (name: string, params: RequestParams = {}) =>
      this.request<WorkflowDetailResponse, void>({
        path: `/api/v1/workflows/${name}`,
        method: "GET",
        secure: true,
        format: "json",
        ...params,
      }),
  };
}
