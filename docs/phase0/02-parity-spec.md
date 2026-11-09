# Phase 0.2 Behavioral parity spec

Goal. Port the provider-facing part of crossplane-runtime v2.4.3 to Rust and prove parity with the Go stack by differential e2e. This document states what the Go code does, branch by branch, so the port and the audit-log normalizer can be written against it.

Status. Derived by reading source. Nothing in this document was executed against a cluster. Statements marked **[inferred]** follow from reading two or more sites together. Statements marked **[unverified]** were not checked in source.

## 0. Conventions

### 0.1 Path roots

| Alias | Root |
|---|---|
| (none) | crossplane-runtime v2.4.3 checkout (SNAP). Example `pkg/meta/meta.go:38`. |
| `XPAPIS/` | `github.com/crossplane/crossplane/apis/v2@v2.4.3` in the Go module cache. |
| `BASE/` | The baseline Go provider (`provider-baseline`). |
| `CR/` | `sigs.k8s.io/controller-runtime@v0.24.1` (pinned by `BASE/go.mod:17`). |
| `CG/` | `k8s.io/client-go@v0.36.3` (pinned by `BASE/go.mod:16`). |
| `AM/` | `k8s.io/apimachinery@v0.36.3`. |

### 0.2 File aliases used in tables

| Alias | File |
|---|---|
| `R:n` | `pkg/reconciler/managed/reconciler.go:n` |
| `A:n` | `pkg/reconciler/managed/api.go:n` |
| `P:n` | `pkg/reconciler/managed/policies.go:n` |
| `MT:n` | `pkg/reconciler/managed/reconciler_modern_test.go:n` |
| `LT:n` | `pkg/reconciler/managed/reconciler_legacy_test.go:n` |
| `RA:n` | `pkg/resource/api.go:n` |
| `RR:n` | `pkg/resource/resource.go:n` |
| `C:n` | `XPAPIS/core/v2/condition.go:n` |
| `PC:n` | `pkg/reconciler/providerconfig/reconciler.go:n` |
| `PT:n` | `pkg/reconciler/providerconfig/reconciler_test.go:n` |
| `PU:n` | `pkg/resource/providerconfig.go:n` |
| `PUT:n` | `pkg/resource/providerconfig_test.go:n` |

### 0.3 Notation

| Notation | Meaning |
|---|---|
| `Synced=False/ReconcileError("m")` | Condition type `Synced`, status `False`, reason `ReconcileError`, message `m`. Built by `ReconcileError(err)` at `C:308-316`. |
| `Synced=True/ReconcileSuccess` | `C:295-302`. No message. |
| `Synced=False/ReconcilePaused` | `C:320-327`. No message. |
| `Ready=False/Creating` | `C:249-256`. No message. |
| `Ready=False/Deleting` | `C:260-267`. No message. |
| `W(reason, msg)` | Warning event. Message is `err.Error()` (`pkg/event/event.go:62-72`). |
| `N(reason, msg)` | Normal event (`pkg/event/event.go:49-59`). |
| `{}` | `reconcile.Result{}` with nil error unless stated. |
| `Wrap(e, "m")` | Message text is `"m: " + e.Error()` (`pkg/errors/errors.go:84-90`, `pkg/errors/errors.go:103-105`). |
| `<err>` | The text of the error returned by the failing collaborator. |
| nil passthrough | `Wrap(nil, m)` is nil (`pkg/errors/errors.go:84-87`). Every `errors.Wrap(updateStatus(), ...)` return yields a nil error when the status update succeeds. |

A citation written as `:n` right after a full citation refers to the same file.

Every condition set by the reconciler gets `observedGeneration = metadata.generation` (`pkg/conditions/manager.go:64-74`, default manager at `R:908`). `SetConditions` keeps the existing entry, including its `lastTransitionTime`, when type, status, reason, message and observedGeneration are equal (`C:114-126`, `C:193-215`).

## 1. Managed reconciler state machine

Entry point is `Reconciler.Reconcile` at `R:919`. A branch is one `return` statement in `Reconcile`. There are 47 `return` statements. `R:938` yields two outcomes, so the table has 48 rows. Rows are in source order. Reaching row N means every earlier guard was false.

### 1.1 Cross-cutting rules

| ID | Rule | Source |
|---|---|---|
| X1 | A deferred call rewrites the final `(result, err)`. If `Cause(err)` is a Kubernetes conflict, the result becomes `{Requeue: true}` and the error is dropped. This applies to every `errors.Wrap(updateStatus(), errUpdateManagedStatus)` return and to `R:938`. | `R:922`, `pkg/errors/reconcile.go:29-35` |
| X2 | `ctx` gets a deadline of `timeout + 30s`. `externalCtx` gets a deadline of `timeout`. Default `timeout` is 1 minute. Connect, Observe, Create, Update and Delete use `externalCtx`. Every Kubernetes call, publish call, ChangeLog call and both Disconnect calls use `ctx`. | `R:49-50`, `R:927-931`, `R:1145`, `R:1178`, `R:1242`, `R:1378`, `R:1539`, `R:1165`, `R:1171` |
| X3 | `updateStatus` sets `status.lastHandledReconcileAt` to the pending token (see S1) and then calls `client.Status().Update(ctx, managed)`. This is a full PUT of the `status` subresource. It is sent even when no condition changed. | `R:999-1006` |
| X4 | If `updateStatus` fails with a non-conflict error, the branch returns its stated result plus error `Wrap(err, "cannot update managed resource status")`. controller-runtime ignores `Requeue` and `RequeueAfter` when the error is non-nil and applies rate-limited backoff. | `A:53`, `CR/pkg/internal/controller/controller.go:491-492` |
| X5 | After a successful Connect, a deferred function runs at every later return. It first calls the connector's `Disconnect(ctx)`, then the external client's `Disconnect(ctx)` if the client is non-nil. Each failure emits `W("CannotDisconnectFromProvider", <err>)`. Neither failure changes the result. | `R:1164-1176` |
| X6 | `recordFirstTimeReconciled` runs after every successful Get. With the real recorder it observes `first_time_to_reconcile` only when `Synced` status is `Unknown`. | `R:941`, `pkg/reconciler/managed/metrics.go:113-118` |
| X7 | The event recorder is annotated with `external-name=<annotation value>` at entry. It is rebuilt with the new value after a successful Create. | `R:944`, `R:1421` |
| X8 | `kerrors.IsConflict` unwraps wrapped errors through `errors.As`. A conflict returned by a default collaborator is therefore detected even though the collaborator wraps it. | `AM/pkg/api/errors/errors.go:811` |

### 1.2 Side branches that do not return

| ID | Guard | Effect | Source | Pinned by |
|---|---|---|---|---|
| S0 | Type switch on `resource.LegacyManaged` | Selects `NewLegacyManagementPoliciesResolver` (uses `spec.deletionPolicy`) or `NewManagementPoliciesResolver`. | `R:961-966` | All `LT` cases use the first arm. All `MT` cases use the second. |
| S1 | Annotation `crossplane.io/reconcile-requested-at` present and non-empty, the object implements `GetLastHandledReconcileAt`/`SetLastHandledReconcileAt`, and the stored value differs | Emits `N("ReconcileRequestHandled", "Handling reconcile request")` with event annotation `token=<value>`. Remembers the token. The token is written to status only by X3. Branches that return without `updateStatus` (B10, B21, B22, B29, every conflict row) do not record it, so the event repeats on the next reconcile. S1 sits after the pause check, so a paused resource never handles a token. | `R:985-994`, `pkg/meta/meta.go:398-404` | `MT:3083` NewReconcileRequestRecordsToken, `MT:3091` AlreadyHandledReconcileRequestIsNoOp. No legacy case. `fake.LegacyManaged` has no `ObservedStatus` (`pkg/resource/fake/mocks.go:399-406`). |
| S2 | `meta.ExternalCreateIncomplete` true and `deterministicExternalName` true | Debug log only. Reconcile continues. | `R:1104`, `R:1113` | `MT:1007`, `LT:1001` CreateSuccessfulAfterExternalCreatePendingAndDeterministicName |
| S3 | `meta.WasDeleted(managed)` true | `ResolveReferences` is skipped. | `R:1125` | Reached by every delete case. Not asserted separately. |
| S4 | Inside B36, `UpdateCriticalAnnotations` fails | Emits `W("CannotUpdateManagedResource", "cannot update managed resource annotations: <err>")`. No condition for this failure. Flow continues to the B36 return. | `R:1399-1408` | Reached by `MT:810`, `LT:804` CreateExternalError (updater returns errBoom at `MT:855`). The success variant is reached by `MT:2683`, `LT:2732` CreateFailureWithChangeLogs. The event is not asserted. |
| S5 | `observation.ResourceLateInitialized && policy.ShouldLateInitialize()` and `client.Update` succeeds | Full PUT of the managed resource. The response overwrites in-memory status. Flow continues to B43 and later rows. | `R:1479-1494` | **Untested.** Only `MT:1076` and `MT:2153` set `ResourceLateInitialized`. The first makes Update fail. The second forbids late init. |
| S6 | `observation.Diff != ""` | Debug log only. | `R:1526-1528` | **Untested.** Not observable outside logs. |
| S7 | Connector-level `Disconnect` fails (X5 first call) | `W("CannotDisconnectFromProvider", <err>)` | `R:1165-1168` | **Untested.** Every table uses `NewNopDisconnector` through `WithExternalConnector` (`R:746-750`). |
| S8 | External client `Disconnect` fails (X5 second call) | `W("CannotDisconnectFromProvider", <err>)` | `R:1170-1175` | `MT:350`, `LT:346` ExternalDisconnectError. Result is still `{RequeueAfter: 1m}`. |

### 1.3 Return branches

Columns. "K8s writes" lists calls made by `Reconcile` itself. Writes made by default collaborators are in section 1.4. "Cond" is what is marked before `updateStatus`. "Event" is the event emitted by this branch. "CL" is a ChangeLog entry. "Metric" is a `MetricRecorder` call.

| ID | Line | Guard | K8s writes by Reconcile | Cond | Event | CL / Metric | Result | Modern cases | Legacy cases |
|---|---|---|---|---|---|---|---|---|---|
| B01 | `R:938` | `client.Get` fails, not NotFound | none | none | none | none | `{}`, error `Wrap(err, "cannot get managed resource")` | `MT:67` GetManagedError | `LT:67` |
| B02 | `R:938` | `client.Get` returns NotFound | none | none | none | none | `{}` | `MT:78` ManagedNotFound | `LT:78` |
| B03 | `R:971-978` | Annotation `crossplane.io/paused` equals `"true"`, or `policy.IsPaused()` | `Status().Update` called directly, not through X3 | `Synced=False/ReconcilePaused` | ``N("ReconciliationPaused", "Reconciliation is paused either through the `spec.managementPolicies` or the pause annotation")`` with event annotation `annotation=crossplane.io/paused` | none | `{}`. On status error, `Wrap(err, "cannot update managed resource status")`. | `MT:1446` ReconciliationPausedSuccessful, `MT:1476` ManagementPolicyReconciliationPausedSuccessful, `MT:1559` ReconciliationPausedError | `LT:1440`, `LT:1470`, `LT:1553` |
| B04 | `R:1019-1020` | `policy.Validate()` error is a conflict | none | none | none | none | `{Requeue: true}` | **Untested. Unreachable.** `Validate` returns `fmt.Errorf` values only (`P:158`, `P:171`). | **Untested** |
| B05 | `R:1023-1026` | `policy.Validate()` error | X3 | `Synced=False/ReconcileError(<validate message>)`, see section 2.1 | `W("CannotUseInvalidManagementPolicy", <validate message>)` | none | `{}`. No requeue. | `MT:1580` ManagementPoliciesUsedButNotEnabled, `MT:1610` ManagementPolicyNotSupported, `MT:1643` CustomManagementPolicyNotSupported | `LT:1574`, `LT:1604`, `LT:1637` |
| B06 | `R:1047-1048` | Deleted, `!ShouldDelete()`, `UnpublishConnection(ctx, mg, {})` conflict | none | none | none | none | `{Requeue: true}` | **Untested** | **Untested** |
| B07 | `R:1051-1054` | Deleted, `!ShouldDelete()`, `UnpublishConnection` error | X3 | `Ready=False/Deleting`, `Synced=False/ReconcileError(<err>)` | `W("CannotUnpublishConnectionDetails", <err>)` | none | `{Requeue: true}` | `MT:89` UnpublishConnectionDetailsDeletionPolicyOrphan | `LT:89` UnpublishConnectionDetailsDeletionPolicyDeleteOrpahn |
| B08 | `R:1064-1065` | Deleted, `!ShouldDelete()`, `RemoveFinalizer` conflict | none | none | none | none | `{Requeue: true}` | **Untested** | **Untested** |
| B09 | `R:1068-1070` | Deleted, `!ShouldDelete()`, `RemoveFinalizer` error | X3 | `Ready=False/Deleting`, `Synced=False/ReconcileError(<err>)` | **none** | none | `{Requeue: true}` | `MT:130` RemoveFinalizerErrorDeletionPolicyOrphan | `LT:127` |
| B10 | `R:1077-1080` | Deleted, `!ShouldDelete()`, unpublish and finalizer removal succeed | none. No status update. | none | none | `recordDeleted` | `{Requeue: false}` | `MT:167` DeleteSuccessfulDeletionPolicyOrphan | `LT:163` |
| B11 | `R:1089-1090` | `Initialize` conflict | none | none | none | none | `{Requeue: true}` | **Untested** | **Untested** |
| B12 | `R:1093-1096` | `Initialize` error | X3 | `Synced=False/ReconcileError(<err>)` | `W("CannotInitializeManagedResource", <err>)` | none | `{Requeue: true}` | `MT:190` InitializeError | `LT:185` |
| B13 | `R:1104-1110` | `ExternalCreateIncomplete` true and `deterministicExternalName` false | X3 | `Ready=False/Creating`, `Synced=False/ReconcileError("cannot determine creation result - remove the crossplane.io/external-create-pending annotation if it is safe to proceed")` | `W("CannotInitializeManagedResource", same message)` | none | `{Requeue: false}`. The resource stays stuck until a human edits it. | `MT:255` ExternalCreatePending | `LT:251` |
| B14 | `R:1134-1135` | Not deleted, `ResolveReferences` conflict | none | none | none | none | `{Requeue: true}` | **Untested** | **Untested** |
| B15 | `R:1138-1141` | Not deleted, `ResolveReferences` error | X3 | `Synced=False/ReconcileError(<err>)` | `W("CannotResolveResourceReferences", <err>)` | none | `{Requeue: true}` | `MT:290` ResolveReferencesError | `LT:286` |
| B16 | `R:1154-1155` | `Connect` conflict | none | none | none | none | `{Requeue: true}` | **Untested** | **Untested** |
| B17 | `R:1158-1161` | `Connect` error | X3 | `Synced=False/ReconcileError("connect failed: <err>")` | `W("CannotConnectToProvider", <err>)`. Event message is not wrapped. | none | `{Requeue: true}` | `MT:320` ExternalConnectError | `LT:316` |
| B18 | `R:1188-1189` | `Observe` conflict | none | none | none | none | `{Requeue: true}` | **Untested** | **Untested** |
| B19 | `R:1192-1195` | `Observe` error | X3 | `Synced=False/ReconcileError("observe failed: <err>")` | `W("CannotObserveExternalResource", <err>)` | none | `{Requeue: true}` even when a poll interval is set | `MT:391` ExternalObserveError | `LT:387` |
| B20 | `R:1203-1210` | `!ResourceExists && ShouldOnlyObserve()` | X3 | `Synced=False/ReconcileError("observe failed: external resource does not exist")` | `W("CannotObserveExternalResource", "external resource does not exist")` | none | `{RequeueAfter: poll}`, see 1.5 | `MT:1677` ObserveOnlyResourceDoesNotExist | `LT:1671` |
| B21 | `R:1218-1222` | `!ResourceExists` and `external-create-succeeded` is younger than `creationGracePeriod` (default 30s) | none. No status update. | none | `N("PendingExternalResource", "Waiting for external resource existence to be confirmed")` | none | `{Requeue: true}` | `MT:431` CreationGracePeriod | `LT:427` |
| B22 | `R:1234-1238` | Deleted and `len(finalizers) > 1` | none. No status update. | none | none | none | `{Requeue: true}`. External Delete is not called. | `MT:219` ExtraFinalizersDelayDelete | `LT:214` |
| B23 | `R:1243-1259` | Deleted, `ResourceExists && ShouldDelete()`, `Delete` error | X3 | `Ready=False/Deleting`, `Synced=False/ReconcileError("delete failed: <err>")` | `W("CannotDeleteExternalResource", <err>)` | CL `DELETE` with error message | `{Requeue: true}`. No conflict short-circuit. | `MT:463` ExternalDeleteError, `MT:2843` DeleteFailureWithChangeLogs | `LT:459`, `LT:2892` |
| B24 | `R:1269-1278` | Deleted, `ResourceExists && ShouldDelete()`, `Delete` ok | X3 | `Ready=False/Deleting`, `Synced=True/ReconcileSuccess` | `N("DeletedExternalResource", "Successfully requested deletion of external resource")` | CL `DELETE` | `{Requeue: true}`. The finalizer stays. | `MT:513` ExternalDeleteSuccessful, `MT:2799` DeleteSuccessfulWithChangeLogs | `LT:509`, `LT:2848` |
| B25 | `R:1288-1289` | Deleted, `UnpublishConnection(ctx, mg, observation.ConnectionDetails)` conflict | none | none | none | none | `{Requeue: true}` | **Untested** | **Untested** |
| B26 | `R:1292-1295` | Deleted, `UnpublishConnection` error | X3 | `Ready=False/Deleting`, `Synced=False/ReconcileError(<err>)` | `W("CannotUnpublishConnectionDetails", <err>)` | none | `{Requeue: true}` | `MT:563` UnpublishConnectionDetailsDeletionPolicyDeleteError | `LT:559` |
| B27 | `R:1305-1306` | Deleted, `RemoveFinalizer` conflict | none | none | none | none | `{Requeue: true}` | **Untested** | **Untested** |
| B28 | `R:1309-1311` | Deleted, `RemoveFinalizer` error | X3 | `Ready=False/Deleting`, `Synced=False/ReconcileError(<err>)` | **none** | none | `{Requeue: true}` | `MT:615` RemoveFinalizerErrorDeletionPolicyDelete | `LT:609` |
| B29 | `R:1318-1321` | Deleted, external resource gone, unpublish and finalizer removal succeed | none. No status update. | none | none | `recordDeleted` | `{Requeue: false}` | `MT:663` DeleteSuccessfulDeletionPolicyDelete | `LT:657` |
| B30 | `R:1330-1331` | `PublishConnection(observation.ConnectionDetails)` conflict | none | none | none | none | `{Requeue: true}` | **Untested** | **Untested** |
| B31 | `R:1334-1337` | `PublishConnection` (observation) error | X3 | `Synced=False/ReconcileError(<err>)` | `W("CannotPublishConnectionDetails", <err>)` | none | `{Requeue: true}` | `MT:698` PublishObservationConnectionDetailsError, `MT:1724` ObserveOnlyPublishConnectionDetailsError | `LT:692`, `LT:1718` |
| B32 | `R:1346-1347` | `AddFinalizer` conflict | none | none | none | none | `{Requeue: true}` | **Untested** | **Untested** |
| B33 | `R:1350-1352` | `AddFinalizer` error | X3 | `Synced=False/ReconcileError(<err>)` | **none** | none | `{Requeue: true}` | `MT:732` AddFinalizerError | `LT:726` |
| B34 | `R:1368-1369` | `!ResourceExists && ShouldCreate()`. Annotation `external-create-pending=now` set in memory. `client.Update` conflict. | `Update` (failed) | none | none | none | `{Requeue: true}` | **Untested** | **Untested** |
| B35 | `R:1372-1375` | Same guard. `client.Update` error. | `Update` (failed), then X3 | `Ready=False/Creating`, `Synced=False/ReconcileError("cannot update managed resource: <err>")` | `W("CannotUpdateManagedResource", "cannot update managed resource: <err>")` | none | `{Requeue: true}` | `MT:762` UpdateCreatePendingError | `LT:756` |
| B36 | `R:1379-1416` | Pending annotation persisted. `Create` error. | `Update` (pending), then `UpdateCriticalAnnotations` with `external-create-failed=now`, then X3 | `Ready=False/Creating`, `Synced=False/ReconcileError("create failed: <err>")` | `W("CannotCreateExternalResource", <err>)`, skipped when the create error is a conflict (`R:1387-1389`). See S4 for the second event. | CL `CREATE` with error message | `{Requeue: true}`. No conflict short-circuit. | `MT:810` CreateExternalError, `MT:2683` CreateFailureWithChangeLogs | `LT:804`, `LT:2732` |
| B37 | `R:1442-1443` | `Create` ok. `UpdateCriticalAnnotations` with `external-create-succeeded=now` returns a conflict. | `Update` (pending) | none | none | CL `CREATE` already sent at `R:1423` | `{Requeue: true}` | **Untested** | **Untested** |
| B38 | `R:1446-1449` | `Create` ok. `UpdateCriticalAnnotations` error. | `Update` (pending), then X3 | `Ready=False/Creating`, `Synced=False/ReconcileError("cannot update managed resource annotations: <err>")` | `W("CannotUpdateManagedResource", "cannot update managed resource annotations: <err>")` | CL `CREATE` | `{Requeue: true}`. The next reconcile lands in B13 unless the name is deterministic. **[inferred]** from `pkg/meta/meta.go:346-362`. | `MT:861` UpdateCriticalAnnotationsError | `LT:855` |
| B39 | `R:1458-1459` | `PublishConnection(creation.ConnectionDetails)` conflict | as B41 minus X3 | none | none | CL `CREATE` | `{Requeue: true}` | **Untested** | **Untested** |
| B40 | `R:1462-1465` | `PublishConnection` (creation) error | `Update` (pending), critical annotations, X3 | `Ready=False/Creating`, `Synced=False/ReconcileError(<err>)` | `W("CannotPublishConnectionDetails", <err>)` | CL `CREATE` | `{Requeue: true}` | `MT:910` PublishCreationConnectionDetailsError | `LT:904` |
| B41 | `R:1472-1476` | Create flow succeeds | `Update` (pending), critical annotations (succeeded), X3 | `Ready=False/Creating`, `Synced=True/ReconcileSuccess` | `N("CreatedExternalResource", "Successfully requested creation of external resource")` | CL `CREATE` | `{Requeue: true}` | `MT:972` CreateSuccessful, `MT:1007`, `MT:1880` ManagementPolicyAllCreateSuccessful, `MT:1922` ManagementPolicyCreateCreateSuccessful, `MT:2645` CreateSuccessfulWithChangeLogs | `LT:966`, `LT:1001`, `LT:1874`, `LT:1916`, `LT:2694` |
| B42 | `R:1487-1492` | `ResourceLateInitialized && ShouldLateInitialize()`, `client.Update` error | `Update` (failed), then X3 | `Synced=False/ReconcileError("cannot update managed resource: <err>")` | `W("CannotUpdateManagedResource", <err>)`. Event message is not wrapped. | none | `{Requeue: true}`. No conflict short-circuit. | `MT:1048` LateInitializeUpdateError | `LT:1042` |
| B43 | `R:1496-1514` | `ResourceUpToDate` | X3 (after S5 `Update` if late init applied) | `Synced=True/ReconcileSuccess` | none | `recordFirstTimeReady`, `recordUnchanged(name)` | `{RequeueAfter: poll}` | `MT:1090` ExternalResourceUpToDate, `MT:1131` ...WithJitter, `MT:1175` ...WithPollIntervalHook, `MT:1213` ...WithMultiplePollIntervalHooks, `MT:350` ExternalDisconnectError, `MT:1511` ReconciliationResumes, `MT:2118` ManagementPolicySkipLateInitialize, `MT:2986`, `MT:2994`, `MT:3002`, `MT:3010`, `MT:3083`, `MT:3091` | `LT:1084`, `LT:1125`, `LT:1169`, `LT:1207`, `LT:346`, `LT:1505`, `LT:2112` |
| B44 | `R:1517-1523` | `!ResourceExists` and create was not allowed | X3 | `Synced=True/ReconcileSuccess` | none | none | `{RequeueAfter: poll}` | `MT:1827` ObserveUpdateResourceDoesNotExist, `MT:2167` ObserveAndLateInitializePolicy, `MT:2193` ObserveUpdateAndLateInitializePolicy | `LT:1821`, `LT:2161`, `LT:2187` |
| B45 | `R:1531-1536` | Exists, not up to date, `!ShouldUpdate()` | X3 | `Synced=True/ReconcileSuccess` | none | none | `{RequeueAfter: poll}` | `MT:1775` ObserveOnlySuccessfulObserve, `MT:1964` ManagementPolicyImmutable | `LT:1769`, `LT:1958` |
| B46 | `R:1539-1555` | `Update` error | X3 | `Synced=False/ReconcileError("update failed: <err>")` | `W("CannotUpdateExternalResource", <err>)` | CL `UPDATE` with error message | `{Requeue: true}`. No conflict short-circuit. | `MT:1255` UpdateExternalError, `MT:2760` UpdateFailureWithChangeLogs | `LT:1249`, `LT:2809` |
| B47 | `R:1565-1573` | `Update` ok, `PublishConnection(update.ConnectionDetails)` error | X3 | `Synced=False/ReconcileError(<err>)` | `W("CannotPublishConnectionDetails", <err>)` | CL `UPDATE`, `recordDrift` | `{Requeue: true}`. No conflict short-circuit. | `MT:1299` PublishUpdateConnectionDetailsError | `LT:1293` |
| B48 | `R:1581-1586` | `Update` ok | X3 | `Synced=True/ReconcileSuccess` | `N("UpdatedExternalResource", "Successfully requested update of external resource")` | CL `UPDATE`, `recordDrift` | `{RequeueAfter: poll}` | `MT:1356` UpdateSuccessful, `MT:1400` TypedReconcilerUpdateSuccessful, `MT:2016` ManagementPolicyAllUpdateSuccessful, `MT:2067` ManagementPolicyUpdateUpdateSuccessful, `MT:2722` UpdateSuccessfulWithChangeLogs | `LT:1350`, `LT:1394`, `LT:2010`, `LT:2061`, `LT:2771` |

Notes on the table.

1. `policy.ShouldDelete()` at `R:1241` is always true when reached. The earlier guard at `R:1032` already returned for every deleted resource where it is false.
2. The reconciler never sets `Ready=True`. The external client must set `Available` in Observe. The reconciler never sets the `UpToDate` condition type declared at `C:49-52`. A search of `pkg/reconciler/managed/reconciler.go` finds no use of `ObserveMatched`, `UpdateRequested`, `UpdateRestricted`, `UpdateFailed` or `PausedUnknown`.
3. What the tables assert. `TestModernReconciler` and `TestReconciler` assert the returned result, the returned error, and the object passed to `Status().Update` (`MT:2225-2236`, condition order ignored through `pkg/test/cmp.go:53-55`). Neither file calls `WithRecorder`, `WithMetricRecorder` or `WithLogger`. **Every event and every metric call in the table is unpinned by tests.** ChangeLog calls are pinned only by `TestReconcilerChangeLogs` (`MT:2623`) and `TestLegacyReconcilerChangeLogs` (`LT:2672`), which assert call count, operation type and error message (`MT:2890-2906`).
4. Case names do not always match fixtures. `MT:1922` ManagementPolicyCreateCreateSuccessful sets `ManagementActionAll` at `MT:1929`. `MT:2067` ManagementPolicyUpdateUpdateSuccessful sets `ManagementActionAll` at `MT:2074`. Port by fixture.
5. `MT:2167` and `MT:2193` pass no collaborator options. They are the only table cases that run the default `NameAsExternalName` and the default `APIFinalizer.AddFinalizer` through `Reconcile`. Other cases run some defaults too. `MT:167` runs the default unpublish. `MT:2683` runs the default critical annotation updater.
6. X1 (conflict on status update inside `Reconcile`) and X4 (non-conflict status failure) are covered only for B03 by `MT:1559`. The helper itself is covered by `pkg/errors/reconcile_test.go:71`.
7. Legacy-only gaps. The legacy file has no counterpart of `TestEffectivePollInterval`, `TestReconcilePollIntervalAnnotation` or `TestReconcileRequestAnnotation`.

### 1.4 Writes made by default collaborators

The tables in 1.3 stub these. An audit log shows them. Defaults are wired at `R:620-629`.

| Collaborator | When | Kubernetes calls | Error text returned to Reconcile | Source | Tests |
|---|---|---|---|---|---|
| `NameAsExternalName.Initialize` | Before B11. Acts only when `crossplane.io/external-name` is empty. | Sets the annotation to `metadata.name`. `Update` (PUT, main resource). | `"cannot update managed resource: <err>"` | `A:69-77` | `pkg/reconciler/managed/api_test.go:64`, `:78`, `:92` |
| `APISimpleReferenceResolver.ResolveReferences` | Before B14, when not deleted. Acts only if the MR has a `ResolveReferences(ctx, client.Reader)` method. | No write if the object is unchanged after resolution. Otherwise `Patch` with patch type `application/apply-patch+yaml` (`AM/pkg/types/patch.go:28-29`), body a JSON merge diff that always carries `apiVersion` and `kind`, field owner `managed.crossplane.io/api-simple-reference-resolver`, `force=true`. | `"cannot resolve references: <err>"` or `"cannot patch the managed resource via server-side apply: <err>"` | `A:42`, `A:210-233`, `A:237-263` | `pkg/reconciler/managed/api_test.go:407`, `:415`, `:431`, `:447`, `:464`, `:511` |
| `APILocalSecretPublisher.PublishConnection` (modern) | B30, B39, B47 call sites. Runs on every non-delete reconcile. | No call when `writeConnectionSecretToRef` is nil. Otherwise `Get` Secret. NotFound leads to `Create`. Found leads to the two apply options, then `Patch` with `application/merge-patch+json` (`AM/pkg/types/patch.go:26`, `RA:87`) whose body is the full desired Secret. The patch is skipped when current and desired `data` are equal with empty equal to nil. Retries on any wrapped API error with `retry.DefaultRetry`. | `"cannot create or update connection secret: <inner>"` where inner is `"cannot get object: ..."`, `"cannot create object: ..."` or `"cannot patch object: ..."` | `A:145-187`, `RA:53-88`, `RR:231-253` | `pkg/reconciler/managed/api_test.go:283`, `:290`, `:304`, `:331`. `pkg/resource/api_test.go:57`, `:68`, `:82`, `:94`, `:108`, `:124` |
| `APISecretPublisher.PublishConnection` (legacy) | Same call sites. | Same, with the Secret namespace taken from the reference. | Same. | `A:100-128`, `RR:112-122` | `pkg/reconciler/managed/api_test.go:160`, `:167`, `:181`, `:208` |
| `UnpublishConnection` (both) | B06, B25 call sites. | None. Always nil. | n/a | `A:133-135`, `A:192-194` | none |
| `APIFinalizer.AddFinalizer` | Before B32. Acts only when the finalizer is absent. | Appends `finalizer.managedresource.crossplane.io`. `Update` (PUT). | `"cannot update object: <err>"` | `RA:165-173`, `R:47` | `pkg/resource/api_test.go:370`, `:381` |
| `APIFinalizer.RemoveFinalizer` | Before B08 and B27. Acts only when the finalizer is present. | Removes it. `Update` (PUT). NotFound is ignored. | `"cannot update object: <err>"` | `RA:176-184` | `pkg/resource/api_test.go:310`, `:321` |
| `RetryingCriticalAnnotationUpdater` | B36, B37, B38, B41. | `Update` (PUT, main resource). On conflict it does `Get`, re-applies the saved annotation map, and returns the conflict so the loop retries. Retries on every error except `context.Canceled`. Backoff is `retry.DefaultRetry`: 5 steps, 10ms, factor 1.0, jitter 0.1. | `"cannot update critical annotations: <err>"` | `A:283-301`, `CG/util/retry/util.go:28-33` | `pkg/reconciler/managed/api_test.go:567`, `:584`, `:598`, `:618` |

Connection Secret shape.

| Field | Modern | Legacy | Source |
|---|---|---|---|
| `metadata.namespace` | MR namespace | `writeConnectionSecretToRef.namespace` | `RR:99`, `RR:115` |
| `metadata.name` | `writeConnectionSecretToRef.name` | same | `RR:100`, `RR:116` |
| `metadata.ownerReferences` | One entry. `apiVersion`, `kind`, `name`, `uid` of the MR. `controller: true`. `blockOwnerDeletion: true`. | same | `RR:101`, `RR:117`, `pkg/meta/meta.go:99-108`, `pkg/meta/meta.go:122-129` |
| `type` | `connection.crossplane.io/v1alpha1` | same | `RR:45`, `RR:103`, `RR:119` |
| `data` | The `ConnectionDetails` map passed in | same | `A:165`, `A:107` |

An existing Secret is refused when it has a different controller UID, or when it has no controller and its type is not `connection.crossplane.io/v1alpha1` (`RR:337-357`). The refusal surfaces as a publish error.

**[inferred]** Publishing is additive only because the write is a merge patch. The skip test compares whole maps (`A:174`). If Observe returns fewer keys than the Secret holds, a PATCH is sent on every reconcile even though it changes nothing.

Annotations written.

| Key | Value | Writer | Source |
|---|---|---|---|
| `crossplane.io/external-name` | `metadata.name` by default, or whatever `Create` sets | Initializer, then persisted by the critical annotation update | `A:74`, `R:1419-1439` |
| `crossplane.io/external-create-pending` | `time.Now()` as RFC3339, second precision | `client.Update` at `R:1365`. A 409 here is wanted and is not retried. | `R:1363-1365`, `pkg/meta/meta.go:300-302` |
| `crossplane.io/external-create-failed` | same format | Critical annotation updater | `R:1397-1399`, `pkg/meta/meta.go:338-340` |
| `crossplane.io/external-create-succeeded` | same format | Critical annotation updater | `R:1437-1439`, `pkg/meta/meta.go:319-321` |

Create is "incomplete" when `pending` parses and is strictly after the later of `succeeded` and `failed` (`pkg/meta/meta.go:346-362`). An unparsable timestamp reads as zero time (`pkg/meta/meta.go:290-295`).

ChangeLog entry (`pkg/reconciler/managed/changelogger.go:85-123`).

| Field | Value |
|---|---|
| `timestamp` | `timestamppb.Now()` |
| `provider` | The string given to `WithProviderVersion`. Baseline uses `"provider-baseline:" + version` (`BASE/cmd/provider/main.go:152`). |
| `api_version`, `kind`, `name` | From the pre-operation deep copy taken at `R:1229` |
| `external_name` | `crossplane.io/external-name` of that copy. For Create this is the name before `Create` ran. |
| `operation` | `OPERATION_TYPE_CREATE`, `OPERATION_TYPE_UPDATE` or `OPERATION_TYPE_DELETE` |
| `snapshot` | The pre-operation object as a protobuf Struct (`RR:479-501`) |
| `error_message` | `err.Error()` of the external call, unset on success |
| `additional_details` | From the `ExternalCreation`, `ExternalUpdate` or `ExternalDelete` |

The send uses a 10 second timeout and `grpc.WaitForReady(true)` (`pkg/reconciler/managed/changelogger.go:33`, `:116-120`). A send failure is logged at info level with `"cannot record change log entry"` and does not change the reconcile outcome (`R:1252-1254`, `R:69`). The default logger is a no-op (`R:907`). The baseline enables it only with `--enable-changelogs`, default false (`BASE/cmd/provider/main.go:68`, `BASE/internal/controller/mytype/wiring.go:62-64`). Tests are `pkg/reconciler/managed/changelogger_test.go:78` and `:112`.

Prometheus metrics (`pkg/reconciler/managed/metrics.go:60-87`). All are histograms with subsystem `crossplane` and one label `gvk`.

| Name | Observed when | Value | Source |
|---|---|---|---|
| `managed_resource_first_time_to_reconcile_seconds` | X6 | Seconds since `creationTimestamp` | `pkg/reconciler/managed/metrics.go:113-118` |
| `managed_resource_first_time_to_readiness_seconds` | B43, when `Ready` is `True` and the name is still in the first-observation map | Seconds since `creationTimestamp` | `pkg/reconciler/managed/metrics.go:142-154` |
| `managed_resource_deletion_seconds` | B10, B29 | Seconds since `deletionTimestamp` | `pkg/reconciler/managed/metrics.go:138-140` |
| `managed_resource_drift_seconds` | B47, B48, when a prior B43 stored a timestamp for this name | Seconds since that timestamp | `pkg/reconciler/managed/metrics.go:120-136` |

The two internal maps are keyed by `metadata.name` only (`pkg/reconciler/managed/metrics.go:110`, `pkg/reconciler/managed/metrics.go:116`, `pkg/reconciler/managed/metrics.go:121`). Two namespaced resources with the same name share an entry. The exported metric name prefix is **[unverified]**. It depends on how the Prometheus client joins subsystem and name. The baseline also wires `pkg/statemetrics` (`BASE/internal/controller/mytype/wiring.go:70-77`). That package was not read for this spec.

### 1.5 Requeue timing

| Result | Meaning under controller-runtime | Source |
|---|---|---|
| `{}` | The queue forgets the item. No requeue. | `CR/pkg/internal/controller/controller.go:509-512` |
| `{Requeue: true}` | Re-added with the controller rate limiter. | `CR/pkg/internal/controller/controller.go:504-507` |
| `{RequeueAfter: d}` | The queue forgets the item and re-adds it after `d`. | `CR/pkg/internal/controller/controller.go:495-503` |
| non-nil error | Rate-limited re-add. `Requeue` and `RequeueAfter` are ignored. | `CR/pkg/internal/controller/controller.go:491-492` |

The controller rate limiter is per-item exponential, base 1s, cap 60s (`pkg/ratelimiter/default.go:42-44`, selected at `pkg/controller/options.go:83-91`).

`poll` in the table is `pollIntervalHook(managed, effectivePollInterval(managed))` (`R:1207`, `R:1503`, `R:1519`, `R:1532`, `R:1581`).

| Step | Rule | Source | Tests |
|---|---|---|---|
| Base | `pollInterval`, default 1 minute | `R:52`, `R:897` | `MT:2919` NoAnnotationReturnsDefault, `MT:3010` |
| Annotation override | `crossplane.io/poll-interval` parsed with Go `time.ParseDuration`. Ignored when missing, empty, unparsable, zero or negative. | `pkg/meta/meta.go:384-394` | `MT:2925`, `MT:2946`, `MT:2986`, `MT:2994` |
| Floor | An annotation value below `minPollInterval` is replaced by `minPollInterval`. Default floor is 0. The floor does not apply to the base value. | `R:867-875` | `MT:2932`, `MT:2939`, `MT:3002` |
| Hook | Default hook returns its input. Only the last hook option wins. | `R:708-710`, `R:716-720` | `MT:1175`, `MT:1213` |
| Jitter hook | `pollInterval + (rand.Float64() - 0.5) * 2 * jitter`. Uniform in `[-jitter, +jitter)`. Uses `math/rand`. | `R:727-731` | `MT:1131` asserts within 1 second (`MT:1163-1173`) |

The annotation `crossplane.io/reconcile-requested-at` does not change the returned result. It causes a reconcile only because an annotation change passes the `DesiredStateChanged` filter (`pkg/resource/predicates.go:55-70`). See S1 for the status write.

Implicit requeue. Comments in `Reconcile` say a status update "implicitly" requeues (for example `R:1041-1044`). With the baseline wiring this is false. The controller is built with `WithEventFilter(resource.DesiredStateChanged())` (`BASE/internal/controller/mytype/wiring.go:91`). That filter passes update events only for annotation changes other than `external-create-pending` and `external-create-failed`, label changes, and generation changes (`pkg/resource/predicates.go:55-70`, `pkg/resource/predicates.go:91-126`). A status-only write passes none of them (`pkg/resource/predicates_test.go:61` StatusChanged). Requeue therefore comes only from the returned result.

Writes by the reconciler that do pass the filter and trigger an extra reconcile **[inferred]**: the `external-name` annotation write, the `external-create-succeeded` annotation write, and a late-init spec write. A finalizer add or remove changes neither annotations, labels nor generation, so it does not.

The global limiter wraps the reconciler. When it limits a request the inner `Reconcile` is not called and the result is `{RequeueAfter: d}` (`pkg/ratelimiter/reconciler.go:54-63`). Baseline uses a token bucket of 10 per second with burst 100 (`pkg/ratelimiter/default.go:32-34`, `BASE/cmd/provider/main.go:65`, `BASE/cmd/provider/main.go:127`).

## 2. Management policy resolver

### 2.1 Validate

| Feature enabled | Policy set | Result | Source | Tests |
|---|---|---|---|---|
| false | empty, or exactly `{"*"}` | nil | `P:156-162` | `MT:2314` DisabledDefault |
| false | anything else | Error ``"`spec.managementPolicies` is set to a non-default value but the feature is not enabled: %s"`` | `R:58`, `P:157-159` | `MT:2306` DisabledNonDefault, `MT:1580` |
| true | Equal to one of the supported sets | nil | `P:165-169` | `MT:2298`, `MT:2322` |
| true | anything else | Error ``"`spec.managementPolicies` is set to a value(%s) which is not supported. Check docs for supported policies"`` | `R:59`, `P:171` | `MT:2330`, `MT:1610`, `MT:1643` |

`%s` is formatted from `sets.Set.UnsortedList()`. For a policy with more than one element the element order in the message is not stable. The policy slice becomes a set, so duplicates collapse (`P:119`).

The feature is on when the reconciler was built with `WithManagementPolicies()` (`R:829-833`, `R:951`). The supported list can be replaced with `WithReconcilerSupportedManagementPolicies` (`R:837-841`).

### 2.2 Truth table, feature enabled

The 15 default supported sets are at `P:55-110`. O Observe, C Create, U Update, D Delete, L LateInitialize.

| # | Set | Source | IsPaused | ShouldCreate | ShouldUpdate | ShouldLateInitialize | ShouldOnlyObserve | ShouldDelete (modern) |
|---|---|---|---|---|---|---|---|---|
| 1 | `*` | `P:59` | F | T | T | T | F | T |
| 2 | O C U L D | `P:61` | F | T | T | T | F | T |
| 3 | O | `P:64` | F | F | F | F | T | F |
| 4 | (empty) | `P:67` | T | F | F | F | F | F |
| 5 | O C U D | `P:70` | F | T | T | F | F | T |
| 6 | O C U L | `P:73` | F | T | T | T | F | F |
| 7 | O C U | `P:77` | F | T | T | F | F | F |
| 8 | O C D L | `P:80` | F | T | F | T | F | T |
| 9 | O C L | `P:84` | F | T | F | T | F | F |
| 10 | O C D | `P:88` | F | T | F | F | F | T |
| 11 | O C | `P:93` | F | T | F | F | F | F |
| 12 | O D | `P:96` | F | F | F | F | F | T |
| 13 | O U | `P:100` | F | F | T | F | F | F |
| 14 | O L | `P:104` | F | F | F | T | F | F |
| 15 | O U L | `P:108` | F | F | T | T | F | F |

Rules behind the table.

| Method | Enabled | Disabled | Source | Tests |
|---|---|---|---|---|
| `IsPaused` | set is empty | false | `P:176-182` | `MT:2252`, `MT:2260`, `MT:2268` |
| `ShouldCreate` | has Create or `*` | true | `P:186-192` | `MT:2360`, `MT:2367`, `MT:2375`, `MT:2383` |
| `ShouldUpdate` | has Update or `*` | true | `P:196-202` | `MT:2413`, `MT:2420`, `MT:2428`, `MT:2436` |
| `ShouldLateInitialize` | has LateInitialize or `*` | true | `P:206-212` | `MT:2466`, `MT:2473`, `MT:2481`, `MT:2489` |
| `ShouldOnlyObserve` | set equals `{Observe}` | false | `P:217-223` | `MT:2519`, `MT:2526`, `MT:2534` |
| `ShouldDelete` (modern) | has Delete or `*` | true | `P:229-235` | `MT:2568`, `MT:2576`, `MT:2588`, `MT:2600` |

No method checks for Observe. A set without Observe is rejected only because it is absent from the supported list.

### 2.3 Legacy difference and deletion policy

The legacy resolver embeds the modern one and overrides only `ShouldDelete` (`P:39-43`, `P:245-269`). The legacy copies of the other tests are at `LT:2246-2528`.

| Feature | `deletionPolicy` | `managementPolicies` | ShouldDelete | Source | Test |
|---|---|---|---|---|---|
| disabled | `Orphan` | any | F | `P:246-248` | `LT:2562` DeletionOrphan |
| disabled | `Delete` | any | T | `P:246-248` | `LT:2574` DeletionDelete |
| disabled | `""` | any | T | `P:246-248` | none. Reached by `LT:459`, which sets no deletion policy. |
| enabled | `Delete` | has Delete or `*` | T | `P:252-254` | `LT:2586`, `LT:2616` |
| enabled | `Orphan` | not `{"*"}` and has Delete | T | `P:257-259` | `LT:2631` DeletionOrphanManagementActionDelete |
| enabled | `Orphan` | `{"*"}` | F | `P:268` | `LT:2601` DeletionOrphanManagementActionAll |
| enabled | `Delete` | no Delete, not `*` | F | `P:268` | `LT:2646` DeletionDeleteManagementActionNoDelete |
| enabled | `Orphan` | no Delete, not `*` | F | `P:268` | **none** |
| enabled | `""` | `{"*"}` | F | `P:252`, `P:257`, `P:268` | **none** |

The last row differs from the disabled case. An empty deletion policy deletes when the feature is off and orphans when it is on with `*`.

Interaction with the reconciler. `ShouldDelete()` false on a deleted resource takes the early path B06 to B10. The external client is never connected and Observe is never called (`R:1032`). Modern resources have no `deletionPolicy`. Orphaning a modern resource is expressed only by a policy set without Delete.

## 3. Legacy versus modern managed resources

| Aspect | Legacy (cluster scoped) | Modern (namespaced) | Source |
|---|---|---|---|
| Interface | `Managed` + `ConnectionSecretWriterTo` + `ProviderConfigReferencer` + `Orphanable` | `Managed` + `LocalConnectionSecretWriterTo` + `TypedProviderConfigReferencer` | `pkg/resource/interfaces.go:209-225` |
| Policy resolver | `NewLegacyManagementPoliciesResolver` with `GetDeletionPolicy()` | `NewManagementPoliciesResolver` | `R:961-966` |
| Publisher dispatch | `ConnectionPublisher` | `LocalConnectionPublisher`. The type switch tests the local interface first. | `R:631-651` |
| Neither interface | Publish and unpublish fail with `"managed resource does not implement connection details"` | same | `R:73`, `R:637-638`, `R:648-649` |
| Secret namespace | From the reference | MR namespace | `RR:115`, `RR:99` |
| ProviderConfig reference | `*xpv2.Reference` (name) | `*xpv2.ProviderConfigReference` (kind and name) | `pkg/resource/interfaces.go:77-87` |
| Usage tracker | `LegacyProviderConfigUsageTracker` | `ProviderConfigUsageTracker` | `pkg/resource/providerconfig.go:212-258`, `:157-208` |
| External tags | `crossplane-providerconfig` | adds `crossplane-namespace` and `crossplane-providerconfig-kind` | `RR:412-440` |
| Reconcile-request tracking | Depends on the type having `Get/SetLastHandledReconcileAt`, not on legacy or modern | same | `R:132-135` |

Everything else in `Reconcile` is identical. A normalized diff of the two test files shows only the deletion-policy fixtures and the `ShouldDelete` tables differ.

Baseline `MyType`.

| Fact | Evidence |
|---|---|
| Namespaced | `BASE/apis/sample/v1alpha1/mytype_types.go:65` |
| Modern. It embeds `xpv2.ManagedResourceSpec`, whose reference types are `*LocalSecretReference` and `*ProviderConfigReference`. | `BASE/apis/sample/v1alpha1/mytype_types.go:46`, `XPAPIS/core/v2/resource_namespace.go:21`, `BASE/apis/sample/v1alpha1/zz_generated.managed.go:21`, `:26` |
| It has no `GetDeletionPolicy`, so it cannot match the legacy arm. | `BASE/apis/sample/v1alpha1/zz_generated.managed.go:11-46` lists all eight generated methods |
| It does **not** implement the reconcile-request tracker. `Status` is a named field, so `ObservedStatus` methods are not promoted to `*MyType`. S1 never fires for MyType. `status.lastHandledReconcileAt` is never written. | `BASE/apis/sample/v1alpha1/mytype_types.go:52-55`, `:66-72`. A search of `BASE/apis` and `BASE/internal` finds no `LastHandledReconcileAt`. |
| Reconciler options actually set | Connector, logger, poll interval (flag default 1m), API event recorder, management policies (flag default true), metric recorder. No jitter hook, no custom initializer, no deterministic-name flag, no min poll interval. `BASE/internal/controller/mytype/wiring.go:50-68`, `BASE/internal/controller/mytype/external.go:61-63`, `BASE/cmd/provider/main.go:62`, `:67` |
| ProviderConfig is namespaced. Only kind `"ProviderConfig"` works. The tracker runs first and rejects an empty kind. The connector then rejects any other kind. The CRD default for `providerConfigRef.kind` is `ClusterProviderConfig`. An e2e manifest must set the kind. | `BASE/apis/v1alpha1/types.go:62`, `BASE/internal/provider/connector.go:96-98`, `PU:184-186`, `BASE/internal/provider/connector.go:121-123`, `XPAPIS/core/v2/resource_namespace.go:32` |

**[inferred]** Baseline happy path. The default initializer writes `external-name = metadata.name` before Observe (`R:1083` runs before `R:1178`). The baseline Observe reports "does not exist" only when the external name is empty (`BASE/internal/controller/mytype/external.go:84-88`). So Observe reports "exists, not up to date" on the first pass and the reconciler takes B48, not the create path. `External.Create` is not reached unless something clears the annotation. On delete, Observe reports "does not exist" (`BASE/internal/controller/mytype/external.go:69-73`), so the reconciler takes B29 and `External.Delete` is not reached. Confirm this against a real run before using it as the expected trace.

## 4. Other controllers and helpers

### 4.1 ProviderConfig reconciler

Source `pkg/reconciler/providerconfig/reconciler.go`, cited as `PC:n`. Tests `pkg/reconciler/providerconfig/reconciler_test.go`, cited as `PT:n`. Whole-reconcile timeout is 2 minutes (`PC:43`, `PC:153`). There is no conflict handling in this reconciler.

| ID | Line | Guard | Effects | Result | Tests |
|---|---|---|---|---|---|
| PC1 | `PC:157-163` | `Get` fails | none | `{}`, error `Wrap(IgnoreNotFound(err), "cannot get ProviderConfig")`. NotFound gives nil. | `PT:90` GetProviderConfigError, `PT:110` ProviderConfigNotFound |
| PC2 | `PC:174-192` | `List` usages fails. Selector is label `crossplane.io/provider-config=<pc name>`. For a typed usage kind it adds `crossplane.io/provider-config-kind=<pc kind>`. A namespaced ProviderConfig adds `InNamespace`. | `W("UsageAccounting", "cannot list ProviderConfigUsages: <err>")` | `{RequeueAfter: 30s}`, nil error | `PT:130` ListProviderConfigUsageError, `PT:337` ListUsagesScopedToNamespaceWhenNamespaced, `PT:375` ListUsagesNotScopedToNamespaceWhenClusterScoped |
| PC3 | `PC:195-206` | A usage has no controller owner reference and `Delete` fails (NotFound ignored) | `Delete` usage. `W("UsageAccounting", "cannot delete ProviderConfigUsage: <err>")` | `{RequeueAfter: 30s}`, nil error | `PT:150` DeleteProviderConfigUsageError |
| PC3b | `PC:196-208` | Same, `Delete` succeeds | `Delete` usage. The user count drops by one. Flow continues. | n/a | **Untested** |
| PC4 | `PC:214-226` | Deleted and users > 0 | `W("UsageAccounting", "Blocking deletion while usages still exist")`. `status.users = n`. Condition `Terminating=True/InUse` with the same message. `Status().Update`. | `{}`, error `Wrap(err, "cannot update ProviderConfig status")` on failure | `PT:178` BlockDeleteWhileInUse |
| PC5 | `PC:228-233` | Deleted, users = 0, `Update` fails | Removes finalizer `in-use.crossplane.io`. `Update`. | `{RequeueAfter: 30s}`, nil error | `PT:219` RemoveFinalizerError |
| PC6 | `PC:236` | Deleted, users = 0, `Update` ok | Same writes | `{}` | `PT:245` SuccessfulDelete |
| PC7 | `PC:239-244` | Not deleted, `Update` fails | Adds the finalizer in memory. `Update`. | `{RequeueAfter: 30s}`, nil error | `PT:271` AddFinalizerError |
| PC8 | `PC:247-249` | Not deleted, `Update` ok | `Update`, then `status.users = n`, then `Status().Update` | `{}`, error `Wrap(err, "cannot update ProviderConfig status")` on failure | `PT:292` UpdateStatusError, `PT:315` SuccessfulSetUsers |

`Update` at `PC:241` is unconditional. A PUT of the ProviderConfig and a PUT of its status are sent on every reconcile, even when the finalizer is present and the count is unchanged. The `Terminating` condition is set through `SetConditions` directly, so it carries no `observedGeneration` (`PC:65-72`, `PC:223`).

Wiring in the baseline. The controller watches the ProviderConfig and its usages. Usage events map to a request for the referenced config (`BASE/internal/controller/config/config.go:48-55`).

| Usage type | Enqueued request | Source | Tests |
|---|---|---|---|
| Typed, reference kind empty | Treated as kind `ProviderConfig` | `pkg/resource/enqueue_handlers.go:72-77` | `pkg/resource/enqueue_handlers_test.go:134` |
| Typed, handler `Kind` set and different | Dropped | `pkg/resource/enqueue_handlers.go:78-80` | `:103`, `:121` |
| Typed, kind starts with `Cluster` | Name only | `pkg/resource/enqueue_handlers.go:82-83` | `:85` |
| Typed, other | Name plus the usage namespace | `pkg/resource/enqueue_handlers.go:84-86` | `:67` |
| Legacy | Name only | `pkg/resource/enqueue_handlers.go:87-88` | `:53` |
| Anything else | Nothing | `pkg/resource/enqueue_handlers.go:71-90` | `:49` |

### 4.2 ProviderConfigUsage tracker

Source `pkg/resource/providerconfig.go`, cited as `PU:n`. Tests `pkg/resource/providerconfig_test.go`, cited as `PUT:n`. The baseline calls the modern tracker at the top of every Connect (`BASE/internal/provider/connector.go:96-98`). A failure surfaces through B17 as `"connect failed: cannot track ProviderConfig usage: <err>"`.

| Step | Modern | Legacy | Source | Tests |
|---|---|---|---|---|
| Reference nil | Error `"managed resource does not reference a ProviderConfig"`. Satisfies `IsMissingReference`. | same | `PU:179-182`, `PU:234-237` | `PUT:355`, `PUT:260` MissingRef |
| Reference kind empty | Error `"managed resource ProviderConfig reference has no Kind"` | not checked | `PU:184-186` | **Untested** |
| Usage name | MR UID | MR UID | `PU:188`, `PU:239` | |
| Usage namespace | MR namespace | not set | `PU:189` | |
| Labels | `crossplane.io/provider-config=<name>`, `crossplane.io/provider-config-kind=<kind>` | `crossplane.io/provider-config=<name>` | `PU:190`, `PU:240` | |
| Owner reference | The MR, `controller: true`, `blockOwnerDeletion: true` | same | `PU:191`, `PU:241` | |
| `providerConfigRef` | name and kind | name | `PU:192`, `PU:242` | |
| `resourceRef` | apiVersion, kind, name of the MR. No UID. | same | `PU:193-197`, `PU:243-247` | |
| Apply | `APIUpdatingApplicator`. `Get` by name. NotFound leads to `Create`. Found leads to `Update` only when the stored `providerConfigRef` differs. The existing usage must have no controller or the same controller UID. | same | `PU:166`, `PU:199-205`, `RA:104-138`, `RR:305-323` | `PUT:365`, `PUT:270` NopUpdate. `pkg/resource/api_test.go:183-250` |
| GVK source | `mg.GetObjectKind().GroupVersionKind()` of the in-memory MR. It feeds the owner reference and `resourceRef`. The cache reader sets the GVK on objects it returns (`CR/pkg/cache/internal/cache_reader.go:105`). Whether it survives the initializer's `Update` response is **[unverified]**. Check the usage body in a real audit log. | same | `PU:177`, `PU:232` | |
| Errors | "not allowed" is swallowed. Others wrap as `"cannot apply ProviderConfigUsage: <inner>"`. | same | `PU:207`, `PU:257` | `PUT:399`, `PUT:304` ApplyError |

In steady state the tracker costs one `Get` and no write per reconcile. The create path and the changed-reference update path have no dedicated tracker test.

### 4.3 Safe-start: CRD gate reconciler and gate

Sources `pkg/reconciler/customresourcesgate/reconciler.go` and `pkg/gate/gate.go`. Lines are cited in full.

| ID | Guard | Effect | Result | Source | Tests |
|---|---|---|---|---|---|
| G1 | CRD has no `Established=True` condition, or has a deletion timestamp | `gate.Set(gvk, false)` for every version in `spec.versions`, served or not | `{}`, nil | `pkg/reconciler/customresourcesgate/reconciler.go:46-52`, `:76-86` | `pkg/reconciler/customresourcesgate/reconciler_test.go:362`, `:401`, `:139-224` |
| G2 | Otherwise | `gate.Set(gvk, true)` for each served version. Unserved versions are left untouched, not set false. | `{}`, nil | `pkg/reconciler/customresourcesgate/reconciler.go:55-64` | `:321`, `:441`, `:481` |
| G3 | `Setup` with a nil gate | Error `"gate is required"` | error | `pkg/reconciler/customresourcesgate/setup.go:33-35` | **Untested** |
| G4 | `Setup` otherwise | Registers a controller named `crd-gate` for `CustomResourceDefinition` | nil | `pkg/reconciler/customresourcesgate/setup.go:42-45` | **Untested** |

The reconciler makes no Kubernetes writes. It needs get, list and watch on CRDs (`pkg/reconciler/customresourcesgate/reconciler.go:18`). `TransformStripCRDSchema` strips `spec.versions[].schema`, `managedFields` and the annotation `kubectl.kubernetes.io/last-applied-configuration` from cached CRDs (`pkg/reconciler/customresourcesgate/cache.go:43-58`, tests `pkg/reconciler/customresourcesgate/cache_test.go:43-227`). The baseline does not install it (`BASE/cmd/provider/main.go:95-97`).

Gate semantics (`pkg/gate/gate.go`).

| Operation | Rule | Source | Tests |
|---|---|---|---|
| `Register(fn, deps...)` | Appends, then evaluates all registrations. | `pkg/gate/gate.go:45-51` | `pkg/gate/gate_test.go:43`, `:52`, `:61` |
| Release | A registration fires when every dependency is currently true. Zero dependencies fire at once. `fn` runs on a new goroutine. The registration is then removed, so `fn` runs once. | `pkg/gate/gate.go:79-106` | `:102`, `:120`, `:139`, `:157`, `:194` |
| `Set(k, v)` | Returns true when the stored value changed or was absent. Evaluation runs only on a change. | `pkg/gate/gate.go:55-77` | `:175`, `:284`, `:305` |
| Unset before Register | A key set true then false does not satisfy a later registration. | `pkg/gate/gate.go:87-91` | `:175` DependencySetThenUnset |

Baseline use. `main` builds `gate.Gate[schema.GroupVersionKind]`, sets up the CRD gate controller first, then registers `mytype.Setup` behind the MyType GVK. A `Setup` failure inside the callback panics (`BASE/cmd/provider/main.go:129`, `:160-161`, `BASE/internal/controller/mytype/wiring.go:37-44`). The ProviderConfig controller is not gated (`BASE/internal/controller/register.go:33-36`). A once-released controller is not stopped when the CRD later becomes unready.

### 4.4 Late initializer

Source `pkg/resource/late_initializer.go`. Tests `pkg/resource/late_initializer_test.go`. Pure functions. No Kubernetes calls.

| Function | Rule | `changed` set | Source | Tests |
|---|---|---|---|---|
| `LateInitializeStringPtr(org, from)` | Returns `org` when `org != nil` or `from == nil`. Otherwise returns `from`. | Only in the second case | `pkg/resource/late_initializer.go:51-59` | `:45`, `:55`, `:65` |
| `LateInitializeInt64Ptr` | same | same | `pkg/resource/late_initializer.go:62-70` | `:111`, `:121`, `:131` |
| `LateInitializeBoolPtr` | same | same | `pkg/resource/late_initializer.go:73-81` | `:177`, `:187`, `:197` |
| `LateInitializeTimePtr(org *metav1.Time, from *time.Time)` | same, wrapping `*from` in `metav1.Time` | same | `pkg/resource/late_initializer.go:85-95` | `:244`, `:254`, `:264` |
| `IsChanged`, `SetChanged` | Sticky boolean | n/a | `pkg/resource/late_initializer.go:40-48` | via the above |

The reconciler does not call this helper. The external client uses it and reports the outcome through `ExternalObservation.ResourceLateInitialized` (`R:513-525`), which drives S5 and B42.

## 5. Wire-contract constants

| Kind | Name | Value | Source |
|---|---|---|---|
| Finalizer | managed resource | `finalizer.managedresource.crossplane.io` | `R:47` |
| Finalizer | provider config | `in-use.crossplane.io` | `PC:41` |
| Annotation | external name | `crossplane.io/external-name` | `pkg/meta/meta.go:38` |
| Annotation | create pending | `crossplane.io/external-create-pending` | `pkg/meta/meta.go:44` |
| Annotation | create succeeded | `crossplane.io/external-create-succeeded` | `pkg/meta/meta.go:53` |
| Annotation | create failed | `crossplane.io/external-create-failed` | `pkg/meta/meta.go:58` |
| Annotation | paused (value must be `true`) | `crossplane.io/paused` | `pkg/meta/meta.go:65`, `pkg/meta/meta.go:378-380` |
| Annotation | poll interval | `crossplane.io/poll-interval` | `pkg/meta/meta.go:70` |
| Annotation | reconcile request | `crossplane.io/reconcile-requested-at` | `pkg/meta/meta.go:76` |
| Annotation | stripped from cached CRDs | `kubectl.kubernetes.io/last-applied-configuration` | `pkg/reconciler/customresourcesgate/cache.go:55` |
| Label | usage to config name | `crossplane.io/provider-config` | `XPAPIS/core/v2/resource.go:51` |
| Label | usage to config kind | `crossplane.io/provider-config-kind` | `XPAPIS/core/v2/resource.go:47` |
| Status field | handled token | `status.lastHandledReconcileAt` | `XPAPIS/core/v2/observation.go:32` |
| Status field | user count | `status.users` | `XPAPIS/core/v2/resource.go:305` |
| Secret type | connection secret | `connection.crossplane.io/v1alpha1` | `RR:45` |
| Field owner | reference resolver | `managed.crossplane.io/api-simple-reference-resolver` | `A:42` |
| External tag | kind | `crossplane-kind` | `RR:50` |
| External tag | name | `crossplane-name` | `RR:51` |
| External tag | namespace | `crossplane-namespace` | `RR:52` |
| External tag | provider config | `crossplane-providerconfig` | `RR:53` |
| External tag | provider config kind | `crossplane-providerconfig-kind` | `RR:54` |
| Controller name | managed | `"managed/" + lower(kind)` | `R:102-104` |
| Controller name | provider config | `"providerconfig/" + lower(kind)` | `PC:76-78` |
| Controller name | CRD gate | `crd-gate` | `pkg/reconciler/customresourcesgate/setup.go:44` |
| Condition type | | `Ready`, `Synced`, `Healthy`, `UpToDate` | `C:32`, `C:36`, `C:47`, `C:52` |
| Condition type | provider config | `Terminating` | `PC:59` |
| Condition reason | Ready | `Available`, `Unavailable`, `Creating`, `Deleting` | `C:60-63` |
| Condition reason | Synced | `ReconcileSuccess`, `ReconcileError`, `ReconcilePaused` | `C:68-70` |
| Condition reason | UpToDate (unused by the reconciler) | `UpdateRestricted`, `ObserveMatched`, `UpdateFailed`, `UpdateRequested` | `C:75-78` |
| Condition reason | provider config | `InUse` | `PC:60` |
| Event type | | `Normal`, `Warning` | `pkg/event/event.go:33-34` |
| Event reason | | `CannotConnectToProvider` | `R:78` |
| Event reason | | `CannotDisconnectFromProvider` | `R:79` |
| Event reason | | `CannotInitializeManagedResource` | `R:80` |
| Event reason | | `CannotResolveResourceReferences` | `R:81` |
| Event reason | | `CannotObserveExternalResource` | `R:82` |
| Event reason | | `CannotCreateExternalResource` | `R:83` |
| Event reason | | `CannotDeleteExternalResource` | `R:84` |
| Event reason | | `CannotPublishConnectionDetails` | `R:85` |
| Event reason | | `CannotUnpublishConnectionDetails` | `R:86` |
| Event reason | | `CannotUpdateExternalResource` | `R:87` |
| Event reason | | `CannotUpdateManagedResource` | `R:88` |
| Event reason | | `CannotUseInvalidManagementPolicy` | `R:89` |
| Event reason | | `DeletedExternalResource` | `R:91` |
| Event reason | | `CreatedExternalResource` | `R:92` |
| Event reason | | `UpdatedExternalResource` | `R:93` |
| Event reason | | `PendingExternalResource` | `R:94` |
| Event reason | | `ReconciliationPaused` | `R:96` |
| Event reason | | `ReconcileRequestHandled` | `R:97` |
| Event reason | provider config | `UsageAccounting` | `PC:54` |
| Event annotation | every managed event | `external-name` | `R:944` |
| Event annotation | paused event | `annotation` | `R:974` |
| Event annotation | reconcile request event | `token` | `R:990` |
| Event message | | ``Reconciliation is paused either through the `spec.managementPolicies` or the pause annotation`` | `R:973` |
| Event message | | `Handling reconcile request` | `R:990` |
| Event message | | `Waiting for external resource existence to be confirmed` | `R:1220` |
| Event message | | `Successfully requested deletion of external resource` | `R:1275` |
| Event message | | `Successfully requested creation of external resource` | `R:1473` |
| Event message | | `Successfully requested update of external resource` | `R:1583` |
| Event and condition message | provider config | `Blocking deletion while usages still exist` | `PC:216` |
| Error text | status, events | ``"`spec.managementPolicies` is set to a non-default value but the feature is not enabled: %s"`` | `R:58` |
| Error text | status, events | ``"`spec.managementPolicies` is set to a value(%s) which is not supported. Check docs for supported policies"`` | `R:59` |
| Error text | returned | `cannot get managed resource` | `R:61` |
| Error text | status, events | `cannot update managed resource annotations` | `R:62` |
| Error text | status, events | `cannot determine creation result - remove the crossplane.io/external-create-pending annotation if it is safe to proceed` | `R:63` |
| Error text | status prefix | `connect failed` | `R:64` |
| Error text | status prefix | `observe failed` | `R:65` |
| Error text | status prefix | `create failed` | `R:66` |
| Error text | status prefix | `update failed` | `R:67` |
| Error text | status prefix | `delete failed` | `R:68` |
| Error text | log only | `cannot record change log entry` | `R:69` |
| Error text | status, events | `external resource does not exist` | `R:71` |
| Error text | status, events | `managed resource does not implement connection details` | `R:73` |
| Error text | status, events | `cannot create or update connection secret` | `A:47` |
| Error text | status, events | `cannot update managed resource` | `A:48` |
| Error text | status, events | `cannot patch the managed resource via server-side apply` | `A:49` |
| Error text | status, events | `cannot marshal the existing object into JSON` | `A:50` |
| Error text | status, events | `cannot marshal the object with the resolved references into JSON` | `A:51` |
| Error text | status, events | `cannot prepare the JSON merge patch for the resolved object` | `A:52` |
| Error text | returned | `cannot update managed resource status` | `A:53` |
| Error text | status, events | `cannot resolve references` | `A:54` |
| Error text | status, events | `cannot update critical annotations` | `A:55` |
| Error text | status, events | `unexpected object type %T` | `pkg/reconciler/managed/reconciler_typed.go:10` |
| Error text | status, events | `cannot update object` | `RA:35` |
| Error text | applicator | `cannot access object metadata`, `cannot create object`, `cannot get object`, `cannot patch object` | `RA:56`, `RA:60`, `RA:72`, `RA:82` |
| Error text | applicator option | `existing object is not controlled by UID %q` | `RR:318` |
| Error text | applicator option | `refusing to modify uncontrolled secret of type %q` | `RR:348` |
| Error text | applicator option | `existing secret is not controlled by UID %q` | `RR:352` |
| Error text | applicator option | `update not allowed` | `RR:390` |
| Error text | tracker | `managed resource does not reference a ProviderConfig` | `PU:41` |
| Error text | tracker | `managed resource ProviderConfig reference has no Kind` | `PU:42` |
| Error text | tracker | `cannot apply ProviderConfigUsage` | `PU:43` |
| Error text | credentials | `cannot extract from environment variable when none specified` | `PU:36` |
| Error text | credentials | `cannot extract from filesystem when no path specified` | `PU:37` |
| Error text | credentials | `cannot extract from secret key when none specified` | `PU:38` |
| Error text | credentials | `cannot get credentials secret` | `PU:39` |
| Error text | credentials | `no extraction handler registered for source: %s` | `PU:40` |
| Error text | provider config | `cannot get ProviderConfig` | `PC:45` |
| Error text | provider config | `cannot list ProviderConfigUsages` | `PC:46` |
| Error text | provider config | `cannot delete ProviderConfigUsage` | `PC:47` |
| Error text | provider config, log only | `cannot update ProviderConfig` | `PC:48` |
| Error text | provider config | `cannot update ProviderConfig status` | `PC:49` |
| Error text | change log | `cannot snapshot managed resource`, `cannot send change log entry` | `pkg/reconciler/managed/changelogger.go:95`, `:122` |
| Error text | CRD gate | `gate is required` | `pkg/reconciler/customresourcesgate/setup.go:34` |
| Duration | reconcile grace period | 30s | `R:49` |
| Duration | reconcile timeout | 1m | `R:50` |
| Duration | default poll interval | 1m | `R:52`, `pkg/controller/options.go:40` |
| Duration | creation grace period | 30s | `R:53` |
| Duration | change log send timeout | 10s | `pkg/reconciler/managed/changelogger.go:33` |
| Duration | provider config short wait | 30s | `PC:42` |
| Duration | provider config timeout | 2m | `PC:43` |
| Duration | controller backoff | 1s base, 60s cap | `pkg/ratelimiter/default.go:43` |
| Rate | global limiter | `rps` per second, burst `rps * 10` | `pkg/ratelimiter/default.go:32-34` |
| Rate | REST client | QPS `rps * 5`, burst `rps * 10` | `pkg/ratelimiter/default.go:48-57` |
| Retry | `retry.DefaultRetry` | 5 steps, 10ms, factor 1.0, jitter 0.1 | `CG/util/retry/util.go:28-33` |
| Feature flag | | `EnableBetaManagementPolicies` | `pkg/feature/features.go:22` |
| Feature flag | | `EnableAlphaChangeLogs` | `pkg/feature/features.go:27` |
| Metric | subsystem, label | `crossplane`, `gvk` | `pkg/reconciler/managed/metrics.go:32-33` |
| Metric | names | see 1.4 | `pkg/reconciler/managed/metrics.go:64`, `:70`, `:76`, `:82` |
| Enum | management actions | `Observe`, `Create`, `Update`, `Delete`, `LateInitialize`, `*` | `XPAPIS/core/v2/policies.go:31-51` |
| Enum | deletion policy | `Orphan`, `Delete` | `XPAPIS/core/v2/policies.go:62-66` |
| Baseline flag defaults | | `--sync 1h`, `--poll 1m`, `--max-reconcile-rate 10`, `--enable-management-policies true`, `--enable-changelogs false`, leader election id `crossplane-leader-election-provider-baseline` | `BASE/cmd/provider/main.go:61-69`, `:107` |

## 6. Test seam

### 6.1 How the Go tables fake the world

| Piece | What it is | Source |
|---|---|---|
| `fake.Manager` | A struct embedding the `manager.Manager` interface with fields `Client`, `Scheme`, `Cache`, `Config`, `RESTMapper`, `Logger`. Only the getters are implemented. Any other manager method panics on the nil embedded interface. | `pkg/resource/fake/mocks.go:528-563` |
| `test.MockClient` | A struct of function fields, one per client method. Each method forwards to its field. | `pkg/test/fake.go:274-296`, `:321-398` |
| Nil field | The tables build `&test.MockClient{...}` literals and set only the fields they expect to be called. Calling an unset field is a nil function call and panics. An unset `MockUpdate` or `MockPatch` therefore pins "this call must not happen". | `pkg/test/fake.go:346-353`, example `MT:71` |
| `NewMockClient()` | Fills every field with a no-op. The reconciler tables do not use it. | `pkg/test/fake.go:300-319` |
| `NewMockGetFn(err, fns...)` | Runs each `ObjectFn` on the object passed to `Get`, then returns `err`. Tables use the `ObjectFn` to populate the fetched object (generation, annotations, deletion timestamp, policies). | `pkg/test/fake.go:95-105`, `MT:3165-3170` |
| `NewMockUpdateFn(err, fns...)` | Runs each `ObjectFn`, then returns `err`. With no `ObjectFn` the object stays as the caller mutated it. | `pkg/test/fake.go:160` |
| `MockSubResourceUpdateFn` | A raw function. Tables build the wanted object inside it and `cmp.Diff` it against the argument. This is where conditions and annotations are asserted. | `pkg/test/fake.go:61`, `MT:101-116` |
| `Status()` | Returns a `MockSubResourceClient` built from `MockStatusCreate`, `MockStatusUpdate`, `MockStatusPatch`. | `pkg/test/fake.go:361-367`, `:400-431` |
| `fake.ModernManaged` | `ObjectMeta` + typed provider config ref + local secret ref + `Manageable` + `ConditionedStatus` + `ObservedStatus`. | `pkg/resource/fake/mocks.go:370-377` |
| `fake.LegacyManaged` | `ObjectMeta` + legacy ref + secret ref + `Manageable` + `Orphanable` + `ConditionedStatus`. No `ObservedStatus`. | `pkg/resource/fake/mocks.go:399-406` |
| `fake.GVK(obj)`, `fake.SchemeWith(objs...)` | Derive a GVK from the Go type and register it in a fresh scheme. `NewReconciler` uses the scheme to construct an empty object per reconcile. | `pkg/resource/fake/mocks.go:569-581`, `R:885-892` |
| `test.EquateErrors()` | Two errors are equal when both are nil, or when their types match and their `Error()` strings match. | `pkg/test/cmp.go:35-49` |
| `test.EquateConditions()` | Sorts condition slices by type before comparing. **[inferred]** go-cmp then uses `Condition.Equal`, which ignores `lastTransitionTime` (`C:114-126`). | `pkg/test/cmp.go:53-55` |
| Function adapters | `ExternalConnectorFn`, `ExternalClientFns`, `InitializerFn`, `ReferenceResolverFn`, `resource.FinalizerFns`, `CriticalAnnotationUpdateFn`, `LocalConnectionPublisherFns`, `ConnectionPublisherFns`. Unset function fields panic when called, which pins "not called" for external operations too. | `R:145-150`, `R:175-217`, `R:249-272`, `R:335-341`, `R:424-459`, `RA:187-200` |
| Test-only options | `withConnectionPublishers`, `withLocalConnectionPublishers` are unexported. | `R:775-788` |
| Change log fake | `changeLogServiceClient` records each `SendChangeLogRequest`. | `pkg/reconciler/managed/changelogger_test.go:44-49` |
| Request | Every table calls `Reconcile(context.Background(), reconcile.Request{})`. The key passed to `Get` is empty and ignored by the mock. | `MT:2229` |

### 6.2 Client surface the in-scope code calls

| Method | Call sites | Notes |
|---|---|---|
| `Get(ctx, key, obj)` | `R:934`, `A:290`, `RA:65`, `RA:117`, `PU:88`, `PC:157`, `BASE/internal/provider/connector.go:128` | Typed objects only. The result is decoded into the passed object. |
| `Update(ctx, obj)` | `R:1365`, `R:1487`, `A:76`, `A:288`, `RA:137`, `RA:172`, `RA:183`, `PC:230`, `PC:241` | Full-object PUT. The response overwrites `obj`, including status and resourceVersion. The code relies on this at `R:1479-1486`. |
| `Status().Update(ctx, obj)` | `R:978`, `R:1005`, `PC:225`, `PC:249` | PUT to the `status` subresource. |
| `Patch(ctx, obj, patch, opts...)` | `A:262` (apply patch type, `FieldOwner`, `ForceOwnership`), `RA:82` (merge patch type, body is the whole desired object) | Two patch types and two option shapes. |
| `Create(ctx, obj)` | `RA:60`, `RA:68`, `RA:111`, `RA:120` | Secrets and usages. |
| `List(ctx, list, opts...)` | `PC:187` | `MatchingLabels` plus optional `InNamespace` (`PC:174-185`). |
| `Delete(ctx, obj)` | `PC:201` | Usages. |
| `client.Reader` | `A:248` | The whole client is handed to the MR's own `ResolveReferences`. |

Not called by in-scope code: `DeleteAllOf`, `Apply`, `SubResource`, `Status().Patch`, `Status().Create`, `RESTMapper`, `GroupVersionKindFor`, `IsObjectNamespaced`.

Manager surface.

| Method | Call sites |
|---|---|
| `GetClient()` | `R:622-627`, `R:895`, `PC:130`, `pkg/reconciler/customresourcesgate/setup.go:45`, `BASE/internal/provider/connector.go:83-84` |
| `GetScheme()` | `R:626-627`, `R:887`, `PC:117`, `PC:121`, `PC:123` |
| `GetEventRecorderFor(name)` | `BASE/internal/controller/mytype/wiring.go:55`, `BASE/internal/controller/config/config.go:55` |
| `Add(runnable)` | `BASE/internal/controller/mytype/wiring.go:74` |
| Controller builder | `BASE/internal/controller/mytype/wiring.go:88-93`, `BASE/internal/controller/config/config.go:48-55`, `pkg/reconciler/customresourcesgate/setup.go:42-45` |

Scheme surface. `New(gvk)` to construct an empty typed object (`RR:126-133`). `ObjectKinds(obj)` to recover the GVK of an MR for owner references (`RR:138-153`, used at `A:106`, `A:164`).

Object surface the reconciler needs from a managed resource. `metav1.Object` accessors, `DeepCopyObject`, `GetObjectKind`, `GetManagementPolicies`, `SetConditions`, `GetCondition` (`pkg/resource/interfaces.go:193-204`). Optional capabilities are discovered by type assertion: legacy (`R:962`), reconcile-request tracker (`R:987`, `R:1001`), local or cluster secret owner (`R:632-639`), reference resolver (`A:238-240`).

### 6.3 What a Rust port needs to port the tables case for case

This is a statement of required seams, not an API design.

1. A Kubernetes client abstraction covering exactly the seven operations in 6.2, where each operation can be replaced per test by a closure, and where an operation with no closure fails the test when called.
2. `Get` must let the test populate the returned object. `Update` must be able to leave the object untouched or overwrite it. The S5 behavior depends on the overwrite.
3. `Status().Update` must hand the test the exact object, so the test can compare conditions (order-insensitive, `lastTransitionTime` excluded), annotations and `lastHandledReconcileAt`.
4. Replaceable collaborators with the same granularity as the Go options: initializer chain, reference resolver, finalizer (add and remove separately), critical annotation updater, local and cluster connection publishers, external connector and client (five operations), change logger, metric recorder, event recorder, poll interval hook.
5. Two fake managed types, one modern with observed status and one legacy with deletion policy and without observed status.
6. An error type that carries a Kubernetes status reason through wrapping, so NotFound and Conflict can be recognized after `Wrap` (X1, X8), and whose display text is `"<message>: <cause>"`.
7. A reconcile result with three distinguishable shapes: none, immediate requeue, requeue after a duration. Tests compare it by value, with a tolerance comparator for jitter.
8. A clock is not injected in Go. Tests compare annotation timestamps formatted at second precision against `time.Now()` taken inside the assertion (`MT:771`, `MT:819-820`). A port that injects a clock can make these cases deterministic. That is a deliberate deviation in test mechanics, not in behavior.

## 7. Non-determinism and timing in an audit-log diff

Each row names something that can differ between two correct providers, or between two runs of one provider.

| # | Source of difference | What shows in the audit log | Evidence | Normalizer hint |
|---|---|---|---|---|
| 1 | Timestamps in annotation values | `external-create-pending`, `-succeeded`, `-failed` carry wall-clock RFC3339 values | `pkg/meta/meta.go:300-302`, `:319-321`, `:338-340` | Mask the values. Keep key presence and ordering of the three. |
| 2 | `lastTransitionTime` | Set from `metav1.Now()` when a condition is constructed. Preserved only when the condition is otherwise equal. | `C:253`, `C:193-215` | Mask the value. Compare type, status, reason, message, observedGeneration. |
| 3 | Status PUT on every reconcile | A status PUT is sent on each poll even with no change. The count of status PUTs depends on how many reconciles ran during the test window. | `R:1005`, `R:1514` | Collapse consecutive identical status PUTs per object. Compare the sequence of distinct states. |
| 4 | Poll interval and jitter | Spacing between reconciles. Jitter is off in the baseline. | `R:727-731`, `BASE/internal/controller/mytype/wiring.go:50-56` | Ignore inter-request timing. |
| 5 | Resync | The informer cache resyncs every `--sync` (default 1h). Resync update events have equal old and new objects and do not pass `DesiredStateChanged`. **[inferred]** | `BASE/cmd/provider/main.go:61`, `:95-97`, `pkg/resource/predicates.go:55-70` | None needed for short suites. |
| 6 | Rate limiters | Per-item backoff 1s to 60s on `Requeue: true` and on errors. Global bucket 10 per second, burst 100. REST client QPS 50, burst 100 with the default flag. Retry counts under error injection differ with timing. | `pkg/ratelimiter/default.go:32-57`, `BASE/cmd/provider/main.go:65`, `:92` | Compare retries as "one or more". |
| 7 | Extra reconciles triggered by own writes | Annotation writes (external-name, create-succeeded) pass the event filter and enqueue another reconcile. Whether that reconcile runs before or after the explicit requeue depends on watch latency. | `pkg/resource/predicates.go:57-66` | Do not assert reconcile counts. |
| 8 | Order of metadata and status writes | Fixed by the code within one reconcile: initializer PUT, then reference-resolver apply PATCH, then (connect) usage GET and maybe CREATE, then Secret GET and CREATE or PATCH for observation details, then finalizer PUT, then create-pending PUT, then critical-annotation PUT, then Secret publish for creation details, then status PUT. Each step is conditional as described in 1.4. | `R:1083`, `R:1126`, `R:1145`, `R:1324`, `R:1340`, `R:1365`, `R:1439`, `R:1452`, `R:1476` | Assert this relative order per object per reconcile. |
| 9 | Update versus patch | Main resource and status use PUT. Reference resolution uses an apply PATCH. Secrets use a merge PATCH. Usages use PUT. A port that switches verbs is visible. | section 6.2 | Compare verb and subresource exactly. |
| 10 | Field managers | Only the reference resolver names one (`managed.crossplane.io/api-simple-reference-resolver`). All other writes pass none, so the API server derives the manager name from the user agent. **[unverified]** for the derivation rule. | `A:262` | Normalize `managedFields[].manager` for non-apply writes. Compare it exactly for the apply PATCH. |
| 11 | User agent | controller-runtime sets `rest.DefaultKubernetesUserAgent()` when none is configured. Its format is `<binary>/<version> (<os>/<arch>) kubernetes/<commit>`. A Rust client sends a different string. | `CR/pkg/manager/manager.go:366-367`, `CG/rest/config.go:525-534` | Drop or map the user agent. Identify the provider by service account instead. |
| 12 | Reads from cache | The manager client reads typed objects from the informer cache by default. GETs and LISTs for the MR, ProviderConfig, usages and Secrets may not appear as individual audit entries. They appear as LIST and WATCH at startup. **[unverified]** against `CR` source at this version. | `CR/pkg/client/client.go:81-87` (cache options exist) | Restrict the diff to write verbs: create, update, patch, delete. |
| 13 | Informers created lazily | A cache-backed GET for a new kind (for example Secret) starts a cluster-wide LIST and WATCH for that kind. **[unverified]** | none read | Same as 12. |
| 14 | Events | Emitted through `GetEventRecorderFor`, the legacy core/v1 recorder. client-go aggregates and rate-limits events, so repeated events become PATCHes that bump `count`. Event names contain a timestamp suffix. **[unverified]** for the aggregation details. | `CR/pkg/manager/internal.go:263-264`, `pkg/event/event.go:105` | Compare the set of (involved object, type, reason, message). Ignore count, names and timing. |
| 15 | Event annotations | Each managed event carries `external-name`. The value changes after Create. | `R:944`, `R:1421` | Keep. It is deterministic for the baseline. |
| 16 | Policy error message order | Set iteration order for multi-element policies. | `P:158`, `P:171` | Sort the bracketed list before comparing, or use single-element policies in e2e. |
| 17 | Critical annotation retry | Up to 5 attempts 10ms apart with 10 percent jitter. On conflict an extra GET. | `A:285-298`, `CG/util/retry/util.go:28-33` | Collapse repeated identical PUTs. |
| 18 | Secret apply retry | The publisher retries on any wrapped API error with the same backoff. | `A:149-150`, `RR:231-235` | Same. |
| 19 | Redundant ProviderConfig writes | One PUT and one status PUT per ProviderConfig reconcile, triggered by every usage event. | `PC:241`, `PC:249` | Collapse identical consecutive writes. A port that skips no-op writes would differ here, so the port must keep them or the normalizer must drop no-op writes on both sides. |
| 20 | Redundant Secret patches | See the note in 1.4. | `A:174` | Same choice as 19. |
| 21 | Conflict handling | A 409 on the create-pending PUT is returned at once as a requeue. A 409 elsewhere is swallowed. Whether a 409 occurs depends on interleaving with the test's own writes. | `R:1368-1369`, `R:922` | Allow 409 responses followed by a retry. |
| 22 | Goroutine start of gated controllers | The gated setup runs on a new goroutine when the CRD becomes established. Controller start order is not fixed. | `pkg/gate/gate.go:98` | Do not assert order across controllers. |
| 23 | Leader election | Off by default. When on it writes Leases. | `BASE/cmd/provider/main.go:59`, `:106-110` | Filter `coordination.k8s.io` leases. |
| 24 | Metrics map keyed by name | Not visible in the audit log. Visible in a metrics diff. | `pkg/reconciler/managed/metrics.go:110` | Out of scope for the audit normalizer. |
| 25 | Baseline debug prints | The baseline external client prints the object on each call. Not in the audit log. | `BASE/internal/controller/mytype/external.go:81` | None. |
| 26 | Request body field order and defaults | Go serializes typed structs in declaration order with `omitempty`. A Rust serializer may order or omit differently. The stored object is the same. **[inferred]** | none | Compare parsed request objects, not bytes. |

## 8. Observations that affect the port

1. All 14 conflict short-circuit returns are untested (B04, B06, B08, B11, B14, B16, B18, B25, B27, B30, B32, B34, B37, B39). B04 is unreachable.
2. Events and metrics are not asserted by any reconciler table case. Parity for them rests on this spec and on e2e.
3. The reconciler never sets the `UpToDate` condition type although the API module declares it.
4. `RemoveFinalizer` and `AddFinalizer` failures emit no event (B09, B28, B33). Neither does a Get failure (B01) or a status update failure (X4). Every other collaborator failure emits one.
5. Event text and condition text differ for the same failure in B17, B19, B20, B23, B36, B42 and B46. The event carries the raw error. The condition carries the wrapped error.
6. `pkg/event/event.go:99-103` drops an event when a filter returns true. The comment at `pkg/event/event.go:87-88` says the opposite. `WithAnnotations` builds a new recorder without the filters (`pkg/event/event.go:110-117`), and `Reconcile` always goes through `WithAnnotations` (`R:944`). Filters given to `NewAPIRecorder` therefore never apply to managed reconciler events. The baseline passes none.
7. The baseline `MyType` cannot record reconcile-request tokens (section 3).
8. The legacy `ShouldDelete` returns false for an empty deletion policy with `*` when the feature is on, and true when it is off (section 2.3).
9. The ProviderConfig reconciler writes on every reconcile and has no conflict handling (section 4.1).
