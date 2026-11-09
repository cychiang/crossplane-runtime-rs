# Phase 0 / 01: Scope of the crossplane-runtime v2.4.3 port

This document fixes what `crossplane-runtime-rs` must cover. It inventories every package in crossplane-runtime v2.4.3. It marks what the xp-provider-gen baseline provider uses. It ends with scope options. The choice among them belongs to the user.

## Citation roots and version basis

Every citation is `root:path:line`. The roots are:

- `rt:` is the runtime snapshot at `scratchpad/xpr-v2.4.3`. `diff -rq` of its `pkg/` and `apis/` against the module-cache copy `github.com/crossplane/crossplane-runtime/v2@v2.4.3` (the version the baseline resolves) reports no differences. The snapshot has no `.git`, so the tag itself was not checked.
- `bl:` is the Go baseline provider at `scratchpad/provider-baseline`.
- `ap:` is `github.com/crossplane/crossplane/apis/v2@v2.4.3` in the Go module cache, the directory `go list -m -f '{{.Dir}}' github.com/crossplane/crossplane/apis/v2` prints inside `bl:`.
- `pt:` is the Go provider-template at `/Users/cychiang/Projects/github.com/crossplane/provider-template`, commit `d3efb9c`. It builds against runtime `v2.5.0-rc.0.0.20260908074656-9b2fb6b1d1ff` and apis `v2.3.4` (pt:go.mod:11, pt:go.mod:12). Its evidence is from a different runtime version.
- `ct:` is `github.com/crossplane/crossplane-tools@v0.0.0-20260715161912-60e57f817ad1`, the angryjet version the baseline pins (bl:go.mod:7, bl:go.mod:25).
- `p0:` is `docs/phase0/` in this repository.

Version skew matters for parity. The runtime's own go.mod pins `crossplane/apis/v2 v2.3.4` (rt:go.mod:9) and `controller-runtime v0.23.1` (rt:go.mod:32). The baseline resolves `crossplane/apis/v2 v2.4.3` (bl:go.mod:12) and `controller-runtime v0.24.1` (bl:go.mod:17), with `k8s.io/client-go v0.36.3` (bl:go.mod:16). The baseline binary is the parity target, so its resolved versions are the ones whose behavior we reproduce.

## Method

The numbers come from these steps. All ran read-only against the roots above.

- Package list, doc line, and imports come from `go list -f ... ./pkg/... ./apis/...` in `rt:`. It reports 40 packages. The other 14 directories under `pkg/` and `apis/` hold no Go package (proto parents and `test-data`/`testdata` trees).
- Non-test lines are raw `wc -l` over each package's `GoFiles`. They include comments and license headers. Generated lines are shown in brackets where a package has them.
- Table cases come from `hack/go-test-inventory.sh` run over every package. For the closure packages it reproduces p0:go-test-inventory.tsv exactly (473 rows). The script counts `"name": {` entries at two tabs of indent inside `func Test...`, plus one row per test with no table. It is a heuristic, not a count of `t.Run` calls.
- Baseline symbol use comes from a `go/packages` pass over `bl:` with full type information. It records `types.Info.Uses` and `Selections` that resolve into the two crossplane modules. It sees methods, fields, and composite-literal keys. It does not see interface satisfaction, which is listed by hand in section 2.
- Runtime cross-package use comes from an AST pass over the closure packages that records `alias.Symbol` selectors into crossplane packages.
- The baseline import closure is p0:go-baseline-import-closure.txt. It lists 17 runtime packages (16 under `pkg/` plus the changelogs proto) and `core/v2`.

## 1. Every package in v2.4.3

Class values: **provider-facing** means a native provider links it at runtime. **test-support** means it exists for tests. **core-or-CLI-only** means only Crossplane core, composition, or the package tooling uses it. "Baseline import" says whether a baseline source file imports the package (direct) or only a runtime package does (transitive). The two "Imported by" columns drop the `pkg/` prefix.

| Package | Purpose | Non-test lines (generated) | Table cases | In closure | Baseline import | Imported by (non-test, in runtime) | Imported by tests of | Class |
|---|---|---|---|---|---|---|---|---|
| `pkg/certificates` | Loads TLS certificates from a given folder. | 70 | 5 | no | n/a | none | none | provider-facing, optional (use unverified) |
| `pkg/conditions` | Enables consistent interactions with an object's status conditions. | 74 | 7 | yes | transitive | `reconciler/managed` | none | provider-facing |
| `pkg/controller` | Configures controller options. | 130 | 0 | yes | direct | `reconciler/customresourcesgate` | none | provider-facing |
| `pkg/errors` | Is a github.com/pkg/errors compatible API for native errors. | 216 | 8 | yes | direct | `certificates`, `fieldpath`, `meta`, `reconciler/managed`, `reconciler/providerconfig`, `reference`, `resource`, `resource/unstructured/composite`, `statemetrics`, `xcrd`, `xpkg`, `xparser`, `xparser/examples`, `xsignature` | `certificates`, `fieldpath`, `meta`, `reconciler/managed`, `reconciler/providerconfig`, `reference`, `resource`, `webhook`, `xcrd`, `xpkg`, `xparser` | provider-facing |
| `pkg/event` | Records Kubernetes events. | 138 | 4 | yes | direct | `controller`, `reconciler/managed`, `reconciler/providerconfig` | none | provider-facing |
| `pkg/feature` | Contains utilities for managing Crossplane features. | 82 | 1 | yes | direct | `controller`, `reconciler/managed` | none | provider-facing |
| `pkg/fieldpath` | Provides utilities for working with field paths. | 1113 | 123 | no | n/a | `resource/unstructured/claim`, `resource/unstructured/composed`, `resource/unstructured/composite` | none | core-or-CLI-only (by in-tree evidence) |
| `pkg/gate` | Contains a gated function callback registration implementation. | 106 | 11 | yes | direct | none | `gate` | provider-facing |
| `pkg/logging` | Provides Crossplane's recommended logging interface. | 156 | 0 | yes | direct | `controller`, `reconciler/customresourcesgate`, `reconciler/managed`, `reconciler/providerconfig`, `statemetrics` | `reconciler/customresourcesgate` | provider-facing |
| `pkg/meta` | Contains functions for dealing with Kubernetes object metadata. | 410 | 71 | yes | direct | `reconciler/managed`, `reconciler/providerconfig`, `reference`, `resource`, `xcrd` | `reconciler/managed`, `reference`, `resource`, `xcrd` | provider-facing |
| `pkg/password` | Contains a simple password generator. | 61 | 1 | no | n/a | none | none | provider-facing, optional (use unverified) |
| `pkg/ratelimiter` | Contains suggested default ratelimiters for Crossplane. | 155 | 3 | yes | direct | `controller` | none | provider-facing |
| `pkg/reconciler` | Provides reconcilers for Crossplane resources. | 18 | 0 | no | n/a | none | none | doc only |
| `pkg/reconciler/customresourcesgate` | Implements a CustomResourceReconciler to report GKVs status to a Gate. | 190 | 20 | yes | direct | none | none | provider-facing |
| `pkg/reconciler/managed` | Provides a reconciler that manages the lifecycle of a resource in an external system. | 2708 | 211 | yes | direct | `controller` | none | provider-facing |
| `pkg/reconciler/providerconfig` | Provides a reconciler that manages the lifecycle of a ProviderConfig. | 250 | 12 | yes | direct | none | none | provider-facing |
| `pkg/reference` | Contains utilities for working with cross-resource references. | 764 | 98 | no | n/a | none | none | provider-facing, commonly used |
| `pkg/resource` | Provides types and functions that can be used to build Kubernetes controllers that reconcile Crossplane resources. | 1697 | 105 | yes | direct | `conditions`, `reconciler/managed`, `reconciler/providerconfig`, `reference`, `statemetrics` | `reconciler/managed`, `reconciler/providerconfig`, `reference` | provider-facing |
| `pkg/resource/fake` | Provides fake Crossplane resources for use in tests. | 738 | 0 | no | n/a | none | `conditions`, `reconciler/managed`, `reconciler/providerconfig`, `reference`, `resource` | test-support |
| `pkg/resource/unstructured` | Contains utilities unstructured Kubernetes objects. | 220 | 20 | yes | transitive | `resource` | none | provider-facing |
| `pkg/resource/unstructured/claim` | Contains an unstructured composite resource claim. | 305 (49) | 17 | no | n/a | none | `resource` | core-or-CLI-only |
| `pkg/resource/unstructured/composed` | Contains an unstructured composed resource. | 226 (49) | 8 | no | n/a | none | `resource` | core-or-CLI-only |
| `pkg/resource/unstructured/composite` | Contains an unstructured composite resource. | 506 (49) | 25 | no | n/a | none | `resource` | core-or-CLI-only |
| `pkg/resource/unstructured/reference` | Contains references to resources. | 62 | 0 | yes | transitive | `resource`, `resource/fake`, `resource/unstructured/claim`, `resource/unstructured/composite` | `resource/unstructured/claim`, `resource/unstructured/composite` | in closure, composition-only via `pkg/resource` |
| `pkg/statemetrics` | Contains utilities for recording Crossplane resource state metrics. | 207 | 0 | yes | direct | `controller` | none | provider-facing |
| `pkg/test` | Implements utilities that can be used to test Kubernetes controllers that reconcile Crossplane resources. | 540 | 0 | no | n/a | none | `certificates`, `conditions`, `errors`, `fieldpath`, `meta`, `ratelimiter`, `reconciler/customresourcesgate`, `reconciler/managed`, `reconciler/providerconfig`, `reference`, `resource`, `resource/unstructured`, `resource/unstructured/composite`, `version`, `webhook`, `xcrd`, `xpkg`, `xparser`, `xparser/examples` | test-support |
| `pkg/version` | Contains utilities for working with semantic versions. | 69 | 7 | no | n/a | `version/fake`, `xpkg` | `xpkg` | core-or-CLI-only |
| `pkg/version/fake` | Contains semantic version mocks. | 63 | 0 | no | n/a | none | `xpkg` | test-support (core) |
| `pkg/webhook` | Contains utilities for building Kubernetes webhooks. | 197 | 14 | no | n/a | none | none | provider-facing, optional (use unverified) |
| `pkg/xcrd` | Generates CustomResourceDefinitions from Crossplane definitions. | 769 | 25 | no | n/a | none | none | core-or-CLI-only |
| `pkg/xpkg` | Contains functionality pertaining to Crossplane packages. | 2300 | 126 | no | n/a | `xfake`, `xparser/yaml` | none | core-or-CLI-only |
| `pkg/xpkg/fake` | Contains mock Crossplane package implementations. | 214 | 0 | no | n/a | none | none | test-support (core) |
| `pkg/xpkg/parser` | Implements a parser for Crossplane packages. | 639 | 29 | no | n/a | `xpkg`, `xparser/examples`, `xparser/yaml` | `xpkg` | core-or-CLI-only |
| `pkg/xpkg/parser/examples` | Contains utilities for parsing examples. | 96 | 4 | no | n/a | `xpkg` | `xpkg` | core-or-CLI-only |
| `pkg/xpkg/parser/yaml` | Contains utilities for reading yaml packages. | 45 | 0 | no | n/a | none | none | core-or-CLI-only |
| `pkg/xpkg/signature` | Implements image signature verification for Crossplane packages. | 449 | 0 | no | n/a | `xpkg` | none | core-or-CLI-only |
| `apis` | Contains Kubernetes API groups. | 18 | 0 | no | n/a | none | none | doc only |
| `apis/changelogs/proto/v1alpha1` | Generated gRPC client and messages for the ChangeLogService (changelog.proto:26). | 550 (550) | 0 | yes | direct | `reconciler/managed` | `reconciler/managed` | provider-facing |
| `apis/pipelineinspector/proto/v1alpha1` | Generated gRPC for a sidecar that captures function pipeline data (pipeline_inspector.proto:27). | 834 (834) | 0 | no | n/a | none | none | core-or-CLI-only (generated gRPC) |
| `apis/proto/v1alpha1` | Generated gRPC for ExternalSecretStorePluginService (ess.proto:22). | 773 (773) | 0 | no | n/a | none | none | core-or-CLI-only (generated gRPC, ESS plugin) |

Totals: 40 packages, 18158 non-test lines, 955 table cases. The 17 closure packages (16 under `pkg/` plus the changelogs proto) hold 7351 lines, 550 of them generated, and 473 cases.

### Packages outside the closure that a native provider may still use

- **`pkg/reference`** (764 lines, 98 cases). Classed provider-facing. No runtime package imports it outside tests. angryjet's generated reference resolvers import it. ct:cmd/angryjet/main.go:63 sets `ReferenceImport` to this package. ct:cmd/angryjet/main.go:599 passes it to `NewResolveReferencesV2` for `zz_generated.resolvers.go` (ct:cmd/angryjet/main.go:73). The managed reconciler's default resolver calls an MR's `ResolveReferences` method if the MR has one (rt:pkg/reconciler/managed/reconciler.go:625, rt:pkg/reconciler/managed/api.go:235 through :237). The baseline has no `zz_generated.resolvers.go` because `MyType` declares no reference fields. Any kind with a cross-resource reference pulls this package in.
- **`pkg/test`** (540 lines, 0 cases). Classed test-support. Tests of 19 runtime packages import it (table above). pt:internal/controller/mytype/mytype_test.go:26 imports it and pt:internal/controller/mytype/mytype_test.go:67 calls `test.EquateErrors` (defined at rt:pkg/test/cmp.go:35). It also defines `MockClient` (rt:pkg/test/fake.go:274). The baseline has no `_test.go` files, so it does not import it.
- **`pkg/resource/fake`** (738 lines, 0 cases). Classed test-support. Tests of `conditions`, `reconciler/managed`, `reconciler/providerconfig`, `reference`, and `resource` import it. It defines the `ModernManaged` and `LegacyManaged` mocks (rt:pkg/resource/fake/mocks.go:370, rt:pkg/resource/fake/mocks.go:399) that the reconciler tests drive.
- **`pkg/resource/unstructured/{claim,composed,composite}`**. Classed core-or-CLI-only. Their only in-closure tie is three compile-time assertions in rt:pkg/resource/interfaces_test.go:44, rt:pkg/resource/interfaces_test.go:45, and rt:pkg/resource/interfaces_test.go:46. Porting `pkg/resource` tests does not need them otherwise.
- **`pkg/fieldpath`** (1113 lines, 123 cases). Only `claim`, `composed`, and `composite` import it. Neither the baseline nor pt: imports it. Provider use is unverified in the permitted sources.
- **`pkg/webhook`** (197 lines, 14 cases), **`pkg/password`** (61 lines, 1 case), **`pkg/certificates`** (70 lines, 5 cases). No runtime package, no baseline file, and no pt: file imports any of them. Their exported entry points are `NewMutator` and `NewValidator` (rt:pkg/webhook/mutator.go:41, rt:pkg/webhook/validator.go:63), `Generate` (rt:pkg/password/password.go:44), and `LoadMTLSConfig` (rt:pkg/certificates/certificates.go:36). Provider use is unverified in the permitted sources.
- **`apis/proto/v1alpha1`** (ESS plugin gRPC). No runtime package imports it. `controller.ESSOptions` still exists (rt:pkg/controller/options.go:94) but holds only TLS settings and no proto type.

## 2. Public API the baseline needs from each closure package

The baseline references runtime symbols in three ways. Each one is a contract the Rust crate must offer.

### 2a. Interfaces the baseline satisfies without naming them

- `External` implements `managed.ExternalClient` (rt:pkg/reconciler/managed/reconciler.go:379, an alias of `TypedExternalClient` at rt:pkg/reconciler/managed/reconciler.go:386). Its methods are `Observe`, `Create`, `Update`, `Delete`, and `Disconnect` (bl:internal/controller/mytype/external.go:65, :118, :139, :157, :169).
- `Connector` implements `managed.ExternalConnector` (rt:pkg/reconciler/managed/reconciler.go:276) through `Connect` (bl:internal/provider/connector.go:90). It is passed to `WithExternalConnector` (bl:internal/controller/mytype/wiring.go:51), which wraps it with a no-op disconnector (rt:pkg/reconciler/managed/reconciler.go:748).
- `*MyType` must be `resource.Managed` (rt:pkg/resource/interfaces.go:200). `NewReconciler` force-casts the scheme object to it (rt:pkg/reconciler/managed/reconciler.go:887). The connector also asserts `resource.ModernManaged` (bl:internal/provider/connector.go:91; interface at rt:pkg/resource/interfaces.go:209).
- `*MyTypeList` must be `resource.ManagedList` (rt:pkg/resource/interfaces.go:228). It is passed to `NewMRStateRecorder` (bl:internal/controller/mytype/wiring.go:72; parameter at rt:pkg/statemetrics/mr_state_metrics.go:92).
- `*ProviderConfig` must be `resource.ProviderConfig` and `*ProviderConfigUsageList` must be `resource.ProviderConfigUsageList`. The providerconfig reconciler force-casts both (rt:pkg/reconciler/providerconfig/reconciler.go:117, rt:pkg/reconciler/providerconfig/reconciler.go:121).
- `*ProviderConfigUsage` must be `resource.TypedProviderConfigUsage` (rt:pkg/resource/interfaces.go:261). `NewProviderConfigUsageTracker` takes it (rt:pkg/resource/providerconfig.go:165, called at bl:internal/provider/connector.go:84). The providerconfig reconciler probes it for `LegacyProviderConfigUsage` (rt:pkg/reconciler/providerconfig/reconciler.go:123).
- The recorder returned by `NewMRStateRecorder` is a controller-runtime `Runnable`. The baseline hands it to `mgr.Add` (bl:internal/controller/mytype/wiring.go:74). Its `Start` is at rt:pkg/statemetrics/mr_state_metrics.go:135.
- `NewMRMetricRecorder` and `NewMRStateMetrics` results must be Prometheus collectors. The baseline registers them with `metrics.Registry.MustRegister` (bl:cmd/provider/main.go:120, bl:cmd/provider/main.go:121). `MRStateMetrics` implements `Describe` and `Collect` (rt:pkg/statemetrics/mr_state_metrics.go:66, rt:pkg/statemetrics/mr_state_metrics.go:75).

### 2b. Methods and fields the baseline touches

- `controller.Options` fields `Logger`, `MaxConcurrentReconciles`, `PollInterval`, `GlobalRateLimiter`, `Features`, `Gate`, `MetricOptions`, `ChangeLogOptions` (bl:cmd/provider/main.go:124 through bl:cmd/provider/main.go:134, bl:cmd/provider/main.go:154). `MetricOptions` fields `PollStateMetricInterval`, `MRMetrics`, `MRStateMetrics` (bl:cmd/provider/main.go:131 through :133). `ChangeLogOptions.ChangeLogger` (bl:cmd/provider/main.go:150).
- `Options.ForControllerRuntime()` (bl:internal/controller/mytype/wiring.go:90, bl:internal/controller/config/config.go:50). `controller.Gate.Register` (bl:internal/controller/mytype/wiring.go:38).
- `feature.Flags.Enable` (bl:cmd/provider/main.go:138) and `Flags.Enabled` (bl:internal/controller/mytype/wiring.go:58).
- `logging.Logger.Info` (bl:cmd/provider/main.go:139) and `Logger.WithValues` (bl:internal/controller/mytype/wiring.go:53).
- `resource.ProviderConfigUsageTracker.Track` (bl:internal/provider/connector.go:96). `ModernManaged.GetProviderConfigReference` (bl:internal/provider/connector.go:116).
- `resource.ProviderConfigKinds` fields `Config`, `Usage`, `UsageList` (bl:internal/controller/config/config.go:43 through :45). `resource.EnqueueRequestForProviderConfig.Kind` (bl:internal/controller/config/config.go:52).
- `managed.ExternalObservation` fields `ResourceExists`, `ResourceUpToDate`, `ConnectionDetails` (bl:internal/controller/mytype/external.go:105, :110, :114). The struct also has `ResourceLateInitialized` and `Diff` (rt:pkg/reconciler/managed/reconciler.go:525, rt:pkg/reconciler/managed/reconciler.go:538). `ExternalCreation` and `ExternalUpdate` carry `ConnectionDetails` and `AdditionalDetails` (rt:pkg/reconciler/managed/reconciler.go:542, rt:pkg/reconciler/managed/reconciler.go:556).

### 2c. Exported top-level symbols per closure package

Each block splits a package's exported top-level declarations three ways. "Baseline" means a baseline file names it. "Other closure packages only" means only another closure package names it across a package boundary; the citation is the first such site. "No qualified cross-package use" does not mean dead. It includes symbols reached from a used symbol inside the same package, such as `managed.Reconciler` (returned by `NewReconciler`) and the default publishers and initializer `NewReconciler` installs (rt:pkg/reconciler/managed/reconciler.go:620 through :628). Methods are not counted here; 2b lists the ones the baseline calls.

#### `apis/changelogs/proto/v1alpha1`

Exported top-level symbols: 19. Baseline uses 1. Other closure packages use 7 more. 11 have no qualified cross-package use.

- Baseline: `NewChangeLogServiceClient` (bl:cmd/provider/main.go:151).
- Other closure packages only: `ChangeLogEntry` (rt:pkg/reconciler/managed/changelogger.go:100), `ChangeLogServiceClient` (rt:pkg/reconciler/managed/changelogger.go:45), `OperationType` (rt:pkg/reconciler/managed/changelogger.go:133), `OperationType_OPERATION_TYPE_CREATE` (rt:pkg/reconciler/managed/reconciler.go:1410), `OperationType_OPERATION_TYPE_DELETE` (rt:pkg/reconciler/managed/reconciler.go:1252), `OperationType_OPERATION_TYPE_UPDATE` (rt:pkg/reconciler/managed/reconciler.go:1548), `SendChangeLogRequest` (rt:pkg/reconciler/managed/changelogger.go:120).
- No qualified cross-package use: `ChangeLogServiceServer`, `ChangeLogService_SendChangeLog_FullMethodName`, `ChangeLogService_ServiceDesc`, `File_apis_changelogs_proto_v1alpha1_changelog_proto`, `OperationType_OPERATION_TYPE_UNSPECIFIED`, `OperationType_name`, `OperationType_value`, `RegisterChangeLogServiceServer`, `SendChangeLogResponse`, `UnimplementedChangeLogServiceServer`, `UnsafeChangeLogServiceServer`.

#### `pkg/conditions`

Exported top-level symbols: 4. Baseline uses 0. Other closure packages use 2 more. 2 have no qualified cross-package use.

- Baseline: none.
- Other closure packages only: `Manager` (rt:pkg/reconciler/managed/reconciler.go:600), `ObservedGenerationPropagationManager` (rt:pkg/reconciler/managed/reconciler.go:908).
- No qualified cross-package use: `ConditionSet`, `ObjectWithConditions`.

#### `pkg/controller`

Exported top-level symbols: 6. Baseline uses 3. Other closure packages use 1 more. 2 have no qualified cross-package use.

- Baseline: `ChangeLogOptions` (bl:cmd/provider/main.go:149), `MetricOptions` (bl:cmd/provider/main.go:130), `Options` (bl:cmd/provider/main.go:123).
- Other closure packages only: `Gate` (rt:pkg/reconciler/customresourcesgate/reconciler.go:36).
- No qualified cross-package use: `DefaultOptions`, `ESSOptions`.

#### `pkg/errors`

Exported top-level symbols: 14. Baseline uses 3. Other closure packages use 3 more. 8 have no qualified cross-package use.

- Baseline: `Errorf` (bl:internal/provider/connector.go:122), `New` (bl:internal/controller/mytype/external.go:121), `Wrap` (bl:internal/controller/mytype/wiring.go:40).
- Other closure packages only: `Cause` (rt:pkg/resource/resource.go:207), `Is` (rt:pkg/reconciler/managed/api.go:286), `SilentlyRequeueOnConflict` (rt:pkg/reconciler/managed/reconciler.go:922).
- No qualified cross-package use: `As`, `Join`, `MultiError`, `Unwrap`, `WithMessage`, `WithMessagef`, `WithSilentRequeueOnConflict`, `Wrapf`.

#### `pkg/event`

Exported top-level symbols: 13. Baseline uses 1. Other closure packages use 6 more. 6 have no qualified cross-package use.

- Baseline: `NewAPIRecorder` (bl:internal/controller/config/config.go:55).
- Other closure packages only: `FilterFn` (rt:pkg/controller/options.go:43), `NewNopRecorder` (rt:pkg/reconciler/managed/reconciler.go:905), `Normal` (rt:pkg/reconciler/managed/reconciler.go:1220), `Reason` (rt:pkg/reconciler/managed/reconciler.go:78), `Recorder` (rt:pkg/reconciler/managed/reconciler.go:605), `Warning` (rt:pkg/reconciler/managed/reconciler.go:1023).
- No qualified cross-package use: `APIRecorder`, `Event`, `NopRecorder`, `Type`, `TypeNormal`, `TypeWarning`.

#### `pkg/feature`

Exported top-level symbols: 4. Baseline uses 3. Other closure packages use 0 more. 1 have no qualified cross-package use.

- Baseline: `EnableAlphaChangeLogs` (bl:cmd/provider/main.go:143), `EnableBetaManagementPolicies` (bl:cmd/provider/main.go:138), `Flags` (bl:cmd/provider/main.go:128).
- Other closure packages only: none.
- No qualified cross-package use: `Flag`.

#### `pkg/gate`

Exported top-level symbols: 1. Baseline uses 1. Other closure packages use 0 more. 0 have no qualified cross-package use.

- Baseline: `Gate` (bl:cmd/provider/main.go:129).
- Other closure packages only: none.
- No qualified cross-package use: none.

#### `pkg/logging`

Exported top-level symbols: 4. Baseline uses 1. Other closure packages use 2 more. 1 have no qualified cross-package use.

- Baseline: `NewLogrLogger` (bl:cmd/provider/main.go:77).
- Other closure packages only: `Logger` (rt:pkg/controller/options.go:50), `NewNopLogger` (rt:pkg/controller/options.go:38).
- No qualified cross-package use: `SetFilteredKlogLogger`.

#### `pkg/meta`

Exported top-level symbols: 38. Baseline uses 3. Other closure packages use 17 more. 18 have no qualified cross-package use.

- Baseline: `GetExternalName` (bl:internal/controller/mytype/external.go:84), `SetExternalName` (bl:internal/controller/mytype/external.go:127), `WasDeleted` (bl:internal/controller/mytype/external.go:69).
- Other closure packages only: `AddAnnotations` (rt:pkg/reconciler/managed/api.go:294), `AddFinalizer` (rt:pkg/reconciler/providerconfig/reconciler.go:239), `AnnotationKeyExternalCreateFailed` (rt:pkg/resource/predicates.go:63), `AnnotationKeyExternalCreatePending` (rt:pkg/reconciler/managed/reconciler.go:63), `AnnotationKeyReconciliationPaused` (rt:pkg/reconciler/managed/reconciler.go:972), `AsController` (rt:pkg/resource/providerconfig.go:191), `ExternalCreateIncomplete` (rt:pkg/reconciler/managed/reconciler.go:1104), `ExternalCreateSucceededDuring` (rt:pkg/reconciler/managed/reconciler.go:1218), `FinalizerExists` (rt:pkg/resource/api.go:166), `GetPollInterval` (rt:pkg/reconciler/managed/reconciler.go:868), `GetReconcileRequest` (rt:pkg/reconciler/managed/reconciler.go:986), `IsPaused` (rt:pkg/reconciler/managed/reconciler.go:971), `RemoveFinalizer` (rt:pkg/reconciler/providerconfig/reconciler.go:228), `SetExternalCreateFailed` (rt:pkg/reconciler/managed/reconciler.go:1397), `SetExternalCreatePending` (rt:pkg/reconciler/managed/reconciler.go:1363), `SetExternalCreateSucceeded` (rt:pkg/reconciler/managed/reconciler.go:1437), `TypedReferenceTo` (rt:pkg/resource/providerconfig.go:191).
- No qualified cross-package use: `AddControllerReference`, `AddLabels`, `AddOwnerReference`, `AnnotationKeyExternalCreateSucceeded`, `AnnotationKeyExternalName`, `AnnotationKeyPollInterval`, `AnnotationKeyReconcileRequestedAt`, `AsOwner`, `GetExternalCreateFailed`, `GetExternalCreatePending`, `GetExternalCreateSucceeded`, `HaveSameController`, `NamespacedNameOf`, `ReferenceTo`, `RemoveAnnotations`, `RemoveLabels`, `SetReconcileRequest`, `WasCreated`.

#### `pkg/ratelimiter`

Exported top-level symbols: 8. Baseline uses 3. Other closure packages use 2 more. 3 have no qualified cross-package use.

- Baseline: `LimitRESTConfig` (bl:cmd/provider/main.go:92), `NewGlobal` (bl:cmd/provider/main.go:127), `NewReconciler` (bl:internal/controller/config/config.go:53).
- Other closure packages only: `NewController` (rt:pkg/controller/options.go:88), `RateLimiter` (rt:pkg/controller/options.go:54).
- No qualified cross-package use: `BucketRateLimiter`, `ControllerRateLimiter`, `Reconciler`.

#### `pkg/reconciler/customresourcesgate`

Exported top-level symbols: 3. Baseline uses 1. Other closure packages use 0 more. 2 have no qualified cross-package use.

- Baseline: `Setup` (bl:cmd/provider/main.go:160).
- Other closure packages only: none.
- No qualified cross-package use: `Reconciler`, `TransformStripCRDSchema`.

#### `pkg/reconciler/managed`

Exported top-level symbols: 104. Baseline uses 19. Other closure packages use 2 more. 83 have no qualified cross-package use.

- Baseline: `ConnectionDetails` (bl:internal/controller/mytype/external.go:114), `ControllerName` (bl:internal/controller/mytype/wiring.go:48), `ExternalClient` (bl:internal/controller/mytype/wiring.go:52), `ExternalCreation` (bl:internal/controller/mytype/external.go:118), `ExternalDelete` (bl:internal/controller/mytype/external.go:157), `ExternalObservation` (bl:internal/controller/mytype/external.go:101), `ExternalUpdate` (bl:internal/controller/mytype/external.go:139), `NewGRPCChangeLogger` (bl:cmd/provider/main.go:150), `NewMRMetricRecorder` (bl:cmd/provider/main.go:117), `NewReconciler` (bl:internal/controller/mytype/wiring.go:86), `ReconcilerOption` (bl:internal/controller/mytype/external.go:61), `WithChangeLogger` (bl:internal/controller/mytype/wiring.go:63), `WithExternalConnector` (bl:internal/controller/mytype/wiring.go:51), `WithLogger` (bl:internal/controller/mytype/wiring.go:53), `WithManagementPolicies` (bl:internal/controller/mytype/wiring.go:59), `WithMetricRecorder` (bl:internal/controller/mytype/wiring.go:67), `WithPollInterval` (bl:internal/controller/mytype/wiring.go:54), `WithProviderVersion` (bl:cmd/provider/main.go:152), `WithRecorder` (bl:internal/controller/mytype/wiring.go:55).
- Other closure packages only: `ChangeLogger` (rt:pkg/controller/options.go:114), `MetricRecorder` (rt:pkg/controller/options.go:105).
- No qualified cross-package use: `APILocalSecretPublisher`, `APISecretPublisher`, `APISimpleReferenceResolver`, `AdditionalDetails`, `ConnectionDetailsFetcher`, `ConnectionPublisher`, `ConnectionPublisherFns`, `CriticalAnnotationUpdateFn`, `CriticalAnnotationUpdater`, `ExternalClientFns`, `ExternalConnectDisconnecterFns`, `ExternalConnectDisconnector`, `ExternalConnectDisconnectorFns`, `ExternalConnecter`, `ExternalConnector`, `ExternalConnectorFn`, `ExternalDisconnecter`, `ExternalDisconnecterFn`, `ExternalDisconnector`, `ExternalDisconnectorFn`, `FinalizerName`, `GRPCChangeLogger`, `GRPCChangeLoggerOption`, `Initializer`, `InitializerChain`, `InitializerFn`, `LegacyManagementPoliciesResolver`, `LocalConnectionPublisher`, `LocalConnectionPublisherFns`, `MRMetricRecorder`, `ManagementPoliciesChecker`, `ManagementPoliciesResolver`, `ManagementPoliciesResolverOption`, `NameAsExternalName`, `NewAPILocalSecretPublisher`, `NewAPISecretPublisher`, `NewAPISimpleReferenceResolver`, `NewLegacyManagementPoliciesResolver`, `NewManagementPoliciesResolver`, `NewNameAsExternalName`, `NewNopDisconnecter`, `NewNopDisconnector`, `NewNopMetricRecorder`, `NewRetryingCriticalAnnotationUpdater`, `NewTypedNopDisconnector`, `NopClient`, `NopConnecter`, `NopConnector`, `NopDisconnecter`, `NopDisconnector`, `NopMetricRecorder`, `PollIntervalHook`, `Reconciler`, `ReferenceResolver`, `ReferenceResolverFn`, `RetryingCriticalAnnotationUpdater`, `TypedExternalClient`, `TypedExternalClientFns`, `TypedExternalConnectDisconnector`, `TypedExternalConnectDisconnectorFns`, `TypedExternalConnecter`, `TypedExternalConnector`, `TypedExternalConnectorFn`, `TypedNopDisconnector`, `WithCreationGracePeriod`, `WithCriticalAnnotationUpdater`, `WithDeterministicExternalName`, `WithExternalConnectDisconnecter`, `WithExternalConnectDisconnector`, `WithExternalConnecter`, `WithFinalizer`, `WithInitializers`, `WithMinPollInterval`, `WithPollIntervalHook`, `WithPollJitterHook`, `WithReconcilerSupportedManagementPolicies`, `WithReferenceResolver`, `WithSendTimeout`, `WithSupportedManagementPolicies`, `WithTimeout`, `WithTypedExternalConnectDisconnecter`, `WithTypedExternalConnectDisconnector`, `WithTypedExternalConnector`.

#### `pkg/reconciler/providerconfig`

Exported top-level symbols: 9. Baseline uses 4. Other closure packages use 0 more. 5 have no qualified cross-package use.

- Baseline: `ControllerName` (bl:internal/controller/config/config.go:40), `NewReconciler` (bl:internal/controller/config/config.go:53), `WithLogger` (bl:internal/controller/config/config.go:54), `WithRecorder` (bl:internal/controller/config/config.go:55).
- Other closure packages only: none.
- No qualified cross-package use: `ReasonInUse`, `Reconciler`, `ReconcilerOption`, `Terminating`, `TypeTerminating`.

#### `pkg/resource`

Exported top-level symbols: 124. Baseline uses 10. Other closure packages use 24 more. 90 have no qualified cross-package use.

- Baseline: `CommonCredentialExtractor` (bl:internal/provider/connector.go:140), `DesiredStateChanged` (bl:internal/controller/mytype/wiring.go:91), `EnqueueRequestForProviderConfig` (bl:internal/controller/config/config.go:52), `Managed` (bl:apis/sample/v1alpha1/zz_generated.managedlist.go:11), `ManagedKind` (bl:internal/controller/mytype/wiring.go:86), `ModernManaged` (bl:internal/provider/connector.go:115), `NewProviderConfigUsageTracker` (bl:internal/provider/connector.go:84), `ProviderConfigKinds` (bl:internal/controller/config/config.go:42), `ProviderConfigUsage` (bl:apis/v1alpha1/zz_generated.pculist.go:11), `ProviderConfigUsageTracker` (bl:internal/provider/connector.go:73).
- Other closure packages only: `AllowUpdateIf` (rt:pkg/reconciler/managed/api.go:111), `Applicator` (rt:pkg/reconciler/managed/api.go:140), `AsProtobufStruct` (rt:pkg/reconciler/managed/changelogger.go:93), `Conditioned` (rt:pkg/conditions/manager.go:29), `ConnectionSecretFor` (rt:pkg/reconciler/managed/api.go:106), `ConnectionSecretMustBeControllableBy` (rt:pkg/reconciler/managed/api.go:110), `ConnectionSecretOwner` (rt:pkg/reconciler/managed/api.go:100), `Finalizer` (rt:pkg/reconciler/managed/reconciler.go:614), `IgnoreNotFound` (rt:pkg/reconciler/managed/reconciler.go:938), `IsAPIErrorWrapped` (rt:pkg/reconciler/managed/api.go:150), `IsNotAllowed` (rt:pkg/reconciler/managed/api.go:118), `LegacyManaged` (rt:pkg/reconciler/managed/reconciler.go:962), `LegacyProviderConfigUsage` (rt:pkg/reconciler/providerconfig/reconciler.go:123), `LocalConnectionSecretFor` (rt:pkg/reconciler/managed/api.go:164), `LocalConnectionSecretOwner` (rt:pkg/reconciler/managed/api.go:158), `ManagedList` (rt:pkg/statemetrics/mr_state_metrics.go:86), `MustCreateObject` (rt:pkg/reconciler/managed/reconciler.go:887), `MustGetKind` (rt:pkg/reconciler/managed/api.go:106), `NewAPIFinalizer` (rt:pkg/reconciler/managed/reconciler.go:623), `NewAPIPatchingApplicator` (rt:pkg/reconciler/managed/api.go:149), `NewApplicatorWithRetry` (rt:pkg/reconciler/managed/api.go:149), `Object` (rt:pkg/conditions/manager.go:28), `ProviderConfig` (rt:pkg/reconciler/providerconfig/reconciler.go:115), `ProviderConfigUsageList` (rt:pkg/reconciler/providerconfig/reconciler.go:119).
- No qualified cross-package use: `APIFinalizer`, `APIPatchingApplicator`, `APIUpdatingApplicator`, `AnnotationChangedPredicate`, `ApplicatorWithRetry`, `ApplyFn`, `ApplyOption`, `AttributeReferencer`, `CanReference`, `Claim`, `ClaimReferencer`, `ClientApplicator`, `Composed`, `ComposedResourcesReferencer`, `Composite`, `CompositeClaim`, `CompositeClaimKind`, `CompositeKind`, `CompositeResourceDeleter`, `CompositeResourceReferencer`, `CompositionReferencer`, `CompositionRevisionReferencer`, `CompositionRevisionSelector`, `CompositionSelector`, `CompositionUpdater`, `ConnectionDetailsPublishedTimer`, `ConnectionSecretWriterTo`, `DefaultFirstN`, `EnvLookupFn`, `EnvironmentConfigReferencer`, `ErrorIs`, `ExternalResourceTagKeyKind`, `ExternalResourceTagKeyName`, `ExternalResourceTagKeyNamespace`, `ExternalResourceTagKeyProvider`, `ExternalResourceTagKeyProviderConfigKind`, `ExtractEnv`, `ExtractFs`, `ExtractSecret`, `FinalizerFns`, `FirstNAndSomeMore`, `GetExternalTags`, `GetKind`, `Ignore`, `IgnoreAny`, `IsAPIError`, `IsConditionTrue`, `IsMissingReference`, `IsNotControllable`, `LateInitializer`, `LegacyComposite`, `LegacyManagedList`, `LegacyProviderConfigUsageTracker`, `LegacyTracker`, `LegacyTrackerFn`, `LocalConnectionSecretWriterTo`, `Manageable`, `ManagedResourceReferencer`, `ModernTracker`, `ModernTrackerFn`, `MustBeControllableBy`, `NewAPIUpdatingApplicator`, `NewLateInitializer`, `NewLegacyProviderConfigUsageTracker`, `NewNopFinalizer`, `NewNotAllowed`, `NewPredicates`, `Orphanable`, `PredicateFn`, `ProviderConfigReferencer`, `RateLimitingInterface`, `ReconciliationObserver`, `ReferenceNotFound`, `ReferenceNotReady`, `ReferenceReady`, `ReferenceStatus`, `ReferenceStatusType`, `ReferenceStatusUnknown`, `RequiredProviderConfigReferencer`, `RequiredTypedProviderConfigReferencer`, `RequiredTypedResourceReferencer`, `SecretTypeConnection`, `StableNAndSomeMore`, `StoreCurrentRV`, `Tracker`, `TrackerFn`, `TypedProviderConfigReferencer`, `TypedProviderConfigUsage`, `UpdateFn`, `UserCounter`.

#### `pkg/resource/unstructured`

Exported top-level symbols: 4. Baseline uses 0. Other closure packages use 1 more. 3 have no qualified cross-package use.

- Baseline: none.
- Other closure packages only: `Wrapper` (rt:pkg/resource/resource.go:487).
- No qualified cross-package use: `ListWrapper`, `NewClient`, `WrapperClient`.

#### `pkg/resource/unstructured/reference`

Exported top-level symbols: 2. Baseline uses 0. Other closure packages use 2 more. 0 have no qualified cross-package use.

- Baseline: none.
- Other closure packages only: `Claim` (rt:pkg/resource/interfaces.go:40), `Composite` (rt:pkg/resource/interfaces.go:163).
- No qualified cross-package use: none.

#### `pkg/statemetrics`

Exported top-level symbols: 7. Baseline uses 2. Other closure packages use 1 more. 4 have no qualified cross-package use.

- Baseline: `NewMRStateMetrics` (bl:cmd/provider/main.go:118), `NewMRStateRecorder` (bl:internal/controller/mytype/wiring.go:71).
- Other closure packages only: `MRStateMetrics` (rt:pkg/controller/options.go:108).
- No qualified cross-package use: `MRStateRecorder`, `NewNopStateRecorder`, `NopStateRecorder`, `StateRecorder`.


## 3. `crossplane/apis/v2/core/v2`

The package has 1510 non-test lines, of which 503 are generated deepcopy (ap:core/v2/zz_generated.deepcopy.go). Its tests have 31 table cases, all in ap:core/v2/condition_test.go. These 31 are not in the 473.

### 3a. What the baseline embeds

- `MyTypeSpec` embeds `xpv2.ManagedResourceSpec` inline (bl:apis/sample/v1alpha1/mytype_types.go:46). That struct (ap:core/v2/resource_namespace.go:21) holds `WriteConnectionSecretToReference *LocalSecretReference`, `ProviderConfigReference *ProviderConfigReference` with CRD default `{"kind": "ClusterProviderConfig", "name": "default"}` (ap:core/v2/resource_namespace.go:32), and `ManagementPolicies` with default `{"*"}` (ap:core/v2/resource_namespace.go:42).
- `MyTypeStatus` embeds `xpv2.ManagedResourceStatus` (bl:apis/sample/v1alpha1/mytype_types.go:53). That struct (ap:core/v2/resource.go:236) inlines `ConditionedStatus` (ap:core/v2/condition.go:162) and `ObservedStatus` (ap:core/v2/observation.go:20). `ObservedStatus` carries `observedGeneration` and `lastHandledReconcileAt`.
- `ProviderConfigStatus` embeds `xpv2.ProviderConfigStatus` (bl:apis/v1alpha1/types.go:27), which is `ConditionedStatus` plus `Users int64` (ap:core/v2/resource.go:301).
- `ProviderCredentials` uses `xpv2.CredentialsSource` (bl:apis/v1alpha1/types.go:42; ap:core/v2/resource.go:243) and `*xpv2.LocalSecretKeySelector` (bl:apis/v1alpha1/types.go:47; ap:core/v2/resource.go:84).
- `ProviderConfigUsage` embeds `xpv2.TypedProviderConfigUsage` inline (bl:apis/v1alpha1/types.go:95). That struct (ap:core/v2/resource.go:320) holds a `ProviderConfigReference` (kind plus name, ap:core/v2/resource.go:214) and a `TypedReference` (ap:core/v2/resource.go:165).
- Behavior the baseline calls: `ConditionedStatus.SetConditions` (ap:core/v2/condition.go:193; called at bl:internal/controller/mytype/external.go:100), `ConditionedStatus.GetCondition` (ap:core/v2/condition.go:180), the condition constructors `Available`, `Creating`, `Deleting` (ap:core/v2/condition.go:271, :249, :260), and `LocalSecretKeySelector.ToSecretKeySelector` (ap:core/v2/resource.go:92; called at bl:internal/provider/connector.go:137).
- The runtime additionally relies on the condition types `TypeReady` and `TypeSynced` (ap:core/v2/condition.go:32, ap:core/v2/condition.go:36), the constructors `ReconcileSuccess`, `ReconcileError`, `ReconcilePaused`, the `ManagementAction` set, the `CredentialsSource*` values, and the provider-config label keys. The per-symbol split is below.

### 3b. Legacy (cluster-scoped) versus modern (namespaced)

The runtime carries two managed-resource shapes. Their spec structs differ in three fields: the secret ref type, the ProviderConfig ref type, and `DeletionPolicy`.

| Aspect | Legacy | Modern |
|---|---|---|
| Spec struct | `ClusterManagedResourceSpec` (ap:core/v2/resource_cluster.go:21) | `ManagedResourceSpec` (ap:core/v2/resource_namespace.go:21) |
| Connection secret ref | `*SecretReference` with namespace (ap:core/v2/resource_cluster.go:27) | `*LocalSecretReference`, name only |
| ProviderConfig ref | `*Reference`, name and policy, default `{"name": "default"}` (ap:core/v2/resource_cluster.go:32) | `*ProviderConfigReference`, kind and name |
| Deletion policy | `DeletionPolicy`, default `Delete` (ap:core/v2/resource_cluster.go:57) | none; `ManagementPolicies` only |
| Runtime interface | `LegacyManaged` = `Managed` + `ConnectionSecretWriterTo` + `ProviderConfigReferencer` + `Orphanable` (rt:pkg/resource/interfaces.go:220), marked deprecated (rt:pkg/resource/interfaces.go:219) | `ModernManaged` = `Managed` + `LocalConnectionSecretWriterTo` + `TypedProviderConfigReferencer` (rt:pkg/resource/interfaces.go:209) |
| Usage record | `ProviderConfigUsage` with `Reference` (ap:core/v2/resource.go:310); `LegacyProviderConfigUsageTracker` (rt:pkg/resource/providerconfig.go:212) | `TypedProviderConfigUsage` (ap:core/v2/resource.go:320); `ProviderConfigUsageTracker` (rt:pkg/resource/providerconfig.go:165) |

The managed reconciler branches on the shape at run time. It picks the legacy policy resolver for `resource.LegacyManaged` and the modern one otherwise (rt:pkg/reconciler/managed/reconciler.go:961 through :966). It picks the connection publisher by `LocalConnectionSecretOwner` versus `ConnectionSecretOwner` (rt:pkg/reconciler/managed/reconciler.go:633, rt:pkg/reconciler/managed/reconciler.go:635). The providerconfig reconciler records whether the usage kind is legacy (rt:pkg/reconciler/providerconfig/reconciler.go:123).

The tests split the same way. rt:pkg/reconciler/managed/reconciler_legacy_test.go has 135 lines that mention `fake.LegacyManaged` and none that mention `fake.ModernManaged`. rt:pkg/reconciler/managed/reconciler_modern_test.go is the reverse, with 138 lines. The inventory gives 90 legacy cases and 98 modern cases. Only the modern file has `TestEffectivePollInterval`, `TestReconcilePollIntervalAnnotation`, and `TestReconcileRequestAnnotation` (rt:pkg/reconciler/managed/reconciler_modern_test.go:2911, :2977, :3070).

**The xp-provider-gen baseline is modern only.** `MyType` embeds `ManagedResourceSpec` (bl:apis/sample/v1alpha1/mytype_types.go:46) and is `scope=Namespaced` (bl:apis/sample/v1alpha1/mytype_types.go:65). The ProviderConfig and its usage are namespaced too (bl:apis/v1alpha1/types.go:62, bl:apis/v1alpha1/types.go:88). The connector rejects anything that is not `ModernManaged` (bl:internal/provider/connector.go:91 through :94). It uses the typed tracker (bl:internal/provider/connector.go:84). A `MyType` created without `spec.providerConfigRef` gets the CRD default kind `ClusterProviderConfig` (ap:core/v2/resource_namespace.go:32, bl:package/crds/sample.example.com_mytypes.yaml:92 through :95). The baseline defines no such kind, and the connector rejects any non-empty kind other than `ProviderConfig` (bl:internal/provider/connector.go:121 through :123). Such an MR fails to connect. A parity test should pin this behavior or the template should change it. The baseline defines no `ClusterProviderConfig` kind, unlike pt: (pt:apis/v1alpha1/types.go:12).

### 3c. Exported top-level symbols of `core/v2`

#### `core/v2`

Exported top-level symbols: 93. Baseline uses 16. Other closure packages use 27 more. 50 have no qualified cross-package use.

- Baseline: `Available` (bl:internal/controller/mytype/external.go:100), `CommonCredentialSelectors` (bl:internal/provider/connector.go:135), `Condition` (bl:apis/sample/v1alpha1/zz_generated.managed.go:11), `ConditionType` (bl:apis/sample/v1alpha1/zz_generated.managed.go:11), `Creating` (bl:internal/controller/mytype/external.go:123), `CredentialsSource` (bl:apis/v1alpha1/types.go:42), `Deleting` (bl:internal/controller/mytype/external.go:162), `LocalSecretKeySelector` (bl:apis/v1alpha1/types.go:47), `LocalSecretReference` (bl:apis/sample/v1alpha1/zz_generated.managed.go:26), `ManagedResourceSpec` (bl:apis/sample/v1alpha1/mytype_types.go:46), `ManagedResourceStatus` (bl:apis/sample/v1alpha1/mytype_types.go:53), `ManagementPolicies` (bl:apis/sample/v1alpha1/zz_generated.managed.go:16), `ProviderConfigReference` (bl:apis/sample/v1alpha1/zz_generated.managed.go:21), `ProviderConfigStatus` (bl:apis/v1alpha1/types.go:27), `TypedProviderConfigUsage` (bl:apis/v1alpha1/types.go:95), `TypedReference` (bl:apis/v1alpha1/zz_generated.pcu.go:16).
- Other closure packages only: `CompositeDeletePolicy` (rt:pkg/resource/interfaces.go:151), `ConditionReason` (rt:pkg/reconciler/providerconfig/reconciler.go:60), `CredentialsSourceEnvironment` (rt:pkg/resource/providerconfig.go:98), `CredentialsSourceFilesystem` (rt:pkg/resource/providerconfig.go:100), `CredentialsSourceInjectedIdentity` (rt:pkg/resource/providerconfig.go:106), `CredentialsSourceNone` (rt:pkg/resource/providerconfig.go:104), `CredentialsSourceSecret` (rt:pkg/resource/providerconfig.go:102), `DeletionDelete` (rt:pkg/reconciler/managed/policies.go:252), `DeletionOrphan` (rt:pkg/reconciler/managed/policies.go:247), `DeletionPolicy` (rt:pkg/reconciler/managed/policies.go:135), `LabelKeyProviderKind` (rt:pkg/reconciler/providerconfig/reconciler.go:179), `LabelKeyProviderName` (rt:pkg/reconciler/providerconfig/reconciler.go:175), `ManagementAction` (rt:pkg/reconciler/managed/policies.go:100), `ManagementActionAll` (rt:pkg/reconciler/managed/policies.go:157), `ManagementActionCreate` (rt:pkg/reconciler/managed/policies.go:191), `ManagementActionDelete` (rt:pkg/reconciler/managed/policies.go:234), `ManagementActionLateInitialize` (rt:pkg/reconciler/managed/policies.go:104), `ManagementActionObserve` (rt:pkg/reconciler/managed/policies.go:100), `ManagementActionUpdate` (rt:pkg/reconciler/managed/policies.go:100), `ReconcileError` (rt:pkg/reconciler/managed/reconciler.go:1024), `ReconcilePaused` (rt:pkg/reconciler/managed/reconciler.go:975), `ReconcileSuccess` (rt:pkg/reconciler/managed/reconciler.go:1276), `Reference` (rt:pkg/resource/interfaces.go:78), `SecretReference` (rt:pkg/resource/interfaces.go:60), `TypeReady` (rt:pkg/reconciler/managed/metrics.go:145), `TypeSynced` (rt:pkg/reconciler/managed/metrics.go:114), `UpdatePolicy` (rt:pkg/resource/interfaces.go:144).
- No qualified cross-package use: `ClusterManagedResourceSpec`, `CompositeDeleteBackground`, `CompositeDeleteForeground`, `ConditionedStatus`, `CredentialsSecretCAKey`, `CredentialsSecretClientCertKey`, `CredentialsSecretClientKeyKey`, `CredentialsSecretEndpointKey`, `CredentialsSecretKubeconfigKey`, `CredentialsSecretPasswordKey`, `CredentialsSecretPortKey`, `CredentialsSecretTokenKey`, `CredentialsSecretUserKey`, `EnvSelector`, `FsSelector`, `IsSystemConditionType`, `NamespacedReference`, `NamespacedSelector`, `NewConditionedStatus`, `ObserveMatched`, `ObservedStatus`, `PausedUnknown`, `Policy`, `ProviderConfigUsage`, `ReasonAvailable`, `ReasonCreating`, `ReasonDeleting`, `ReasonObserveMatched`, `ReasonReconcileError`, `ReasonReconcilePaused`, `ReasonReconcileSuccess`, `ReasonUnavailable`, `ReasonUpdateFailed`, `ReasonUpdateRequested`, `ReasonUpdateRestricted`, `ResolutionPolicy`, `ResolutionPolicyOptional`, `ResolutionPolicyRequired`, `ResolvePolicy`, `ResolvePolicyAlways`, `SecretKeySelector`, `Selector`, `TypeHealthy`, `TypeUpToDate`, `Unavailable`, `UpdateAutomatic`, `UpdateFailed`, `UpdateManual`, `UpdateRequested`, `UpdateRestricted`.


## 4. Code generation the Go side relies on

The baseline runs two generators from bl:apis/generate.go. A third tool, xp-provider-gen, writes the scaffold itself.

### 4a. controller-gen (sigs.k8s.io/controller-tools v0.20.0, bl:go.mod:101)

Invoked with `object:headerFile=... paths=./... crd:crdVersions=v1 output:artifacts:config=../package/crds` (bl:apis/generate.go:26). It reads `+kubebuilder:` and `+groupName` markers, for example `+kubebuilder:object:root=true`, `+kubebuilder:subresource:status`, `+kubebuilder:printcolumn`, and `+kubebuilder:resource:scope=Namespaced,categories=...` (bl:apis/sample/v1alpha1/mytype_types.go:57 through :65), and `+kubebuilder:validation:Enum` (bl:apis/v1alpha1/types.go:41). It also reads markers inside core/v2, such as the `+kubebuilder:default` values in section 3a.

| Output | What it provides |
|---|---|
| bl:apis/sample/v1alpha1/zz_generated.deepcopy.go | `DeepCopyInto` and `DeepCopy` for `MyType`, `MyTypeList`, `MyTypeObservation`, `MyTypeParameters`, `MyTypeSpec`, `MyTypeStatus`, plus `DeepCopyObject` (the `runtime.Object` method) for `MyType` and `MyTypeList` (lines 16 through 129). It calls `ManagedResourceSpec.DeepCopyInto` and `ManagedResourceStatus.DeepCopyInto` from core/v2 (lines 107, 124). |
| bl:apis/v1alpha1/zz_generated.deepcopy.go | The same for `ProviderConfig`, `ProviderConfigList`, `ProviderConfigSpec`, `ProviderConfigStatus`, `ProviderConfigUsage`, `ProviderConfigUsageList`, `ProviderCredentials` (lines 17 through 176). |
| bl:package/crds/sample.example.com_mytypes.yaml | The `MyType` CRD, `scope: Namespaced` (line 21), with OpenAPI schema and defaults such as `providerConfigRef` defaulting to `kind: ClusterProviderConfig, name: default` (lines 92 through 95). |
| bl:package/crds/example.com_providerconfigs.yaml | The `ProviderConfig` CRD, `scope: Namespaced` (line 19). |
| bl:package/crds/example.com_providerconfigusages.yaml | The `ProviderConfigUsage` CRD, `scope: Namespaced` (line 19). |

CRD parity means the Rust side must emit schemas equivalent to these files from its own types. It also needs a deep-copy equivalent wherever Go relies on `DeepCopyObject`, for example `u.of.DeepCopyObject()` in the usage tracker (rt:pkg/resource/providerconfig.go:176). In Rust that is `Clone`.

### 4b. angryjet generate-methodsets (crossplane-tools)

Invoked at bl:apis/generate.go:29. It chooses method sets by which core/v2 struct a type embeds. `MyType` embeds `ManagedResourceSpec`, so the modern core method set applies (ct:cmd/angryjet/main.go:185, matcher at ct:internal/match/match.go:70). Default file names come from flags at ct:cmd/angryjet/main.go:72 through :77.

| Output | What it provides |
|---|---|
| bl:apis/sample/v1alpha1/zz_generated.managed.go | `GetCondition`, `SetConditions` (delegating to `Status`), `Get/SetManagementPolicies`, `Get/SetProviderConfigReference` (`*ProviderConfigReference`), `Get/SetWriteConnectionSecretToReference` (`*LocalSecretReference`) on `*MyType` (lines 11 through 48). Together with `metav1.Object` and `runtime.Object` this makes `MyType` a `ModernManaged`. |
| bl:apis/sample/v1alpha1/zz_generated.managedlist.go | `GetItems() []resource.Managed` on `*MyTypeList` (lines 11 through 17), which makes it a `ManagedList`. |
| bl:apis/v1alpha1/zz_generated.pc.go | `GetCondition`, `SetConditions`, `GetUsers`, `SetUsers` on `*ProviderConfig` (lines 11 through 28), which makes it a `resource.ProviderConfig`. |
| bl:apis/v1alpha1/zz_generated.pcu.go | `Get/SetProviderConfigReference` (value `ProviderConfigReference`) and `Get/SetResourceReference` (`TypedReference`) on `*ProviderConfigUsage` (lines 11 through 28), which makes it a `TypedProviderConfigUsage`. |
| bl:apis/v1alpha1/zz_generated.pculist.go | `GetItems() []resource.ProviderConfigUsage` on `*ProviderConfigUsageList` (lines 11 through 17). |
| `zz_generated.resolvers.go` (not emitted for the baseline) | `ResolveReferences` methods for fields marked as references, importing `pkg/reference` (ct:cmd/angryjet/main.go:73, ct:cmd/angryjet/main.go:599). |

In Rust these are trait impls. How Rust generates them (derive macro, `macro_rules!`, or hand-written impls) is a Phase 1 design question.

### 4c. xp-provider-gen scaffold

Files headed "Code generated by xp-provider-gen" are bl:cmd/provider/main.go:17, bl:internal/provider/connector.go:17, bl:internal/controller/mytype/wiring.go:17, bl:internal/controller/register.go:17, bl:internal/controller/config/config.go:17, bl:apis/register.go:17, bl:apis/v1alpha1/register.go:17, and bl:apis/generate.go:17. They are the template, not the runtime. The Rust provider template must provide an equivalent for each. User-owned stubs are bl:internal/provider/client.go, bl:internal/provider/options.go, and bl:internal/controller/mytype/external.go.

### 4d. protoc output in the runtime

The changelogs client is generated code (rt:apis/changelogs/proto/v1alpha1/changelog.pb.go, 407 lines; rt:apis/changelogs/proto/v1alpha1/changelog_grpc.pb.go, 143 lines) from rt:apis/changelogs/proto/v1alpha1/changelog.proto. Rust replaces it with tonic or prost output from the same `.proto`.


## 5. Third-party behavior the closure depends on

`go list -deps ./cmd/provider` in `bl:` reports 715 non-standard packages. By package count the largest modules are `k8s.io/client-go` (267), `google.golang.org/grpc` (64), `k8s.io/apimachinery` (59), `k8s.io/api` (57), and `sigs.k8s.io/controller-runtime` (45). The table lists only behavior a call site in the baseline or the closure reaches. Behavior inside controller-runtime or client-go beyond the call site is unverified here and needs its own Phase 0 note.

| Dependency (resolved version) | Behavior to reproduce | Call sites |
|---|---|---|
| controller-runtime manager (v0.24.1) | Manager construction, scheme, client, event recorder, runnables, signal-driven start | `ctrl.NewManager` bl:cmd/provider/main.go:92; `mgr.GetScheme` bl:cmd/provider/main.go:114; `mgr.GetClient` bl:internal/provider/connector.go:83; `mgr.Add` bl:internal/controller/mytype/wiring.go:74; `mgr.Start(ctrl.SetupSignalHandler())` bl:cmd/provider/main.go:162; reconcilers take `manager.Manager` (rt:pkg/reconciler/managed/reconciler.go:884, rt:pkg/reconciler/providerconfig/reconciler.go:114) |
| controller-runtime leader election | Lease-only lock, ID `crossplane-leader-election-provider-baseline`, 60s lease, 50s renew deadline, off by default | bl:cmd/provider/main.go:106 through :110; flag default `false` at bl:cmd/provider/main.go:59 |
| controller-runtime cache | Informer resync period from `--sync`, default 1h | bl:cmd/provider/main.go:61, bl:cmd/provider/main.go:95 through :97. `customresourcesgate.TransformStripCRDSchema` exists for a CRD cache transform (rt:pkg/reconciler/customresourcesgate/cache.go:43) but the baseline does not install it |
| controller-runtime client | Get, Update, Status().Update, Patch on typed objects | e.g. rt:pkg/reconciler/managed/reconciler.go:934, rt:pkg/reconciler/managed/reconciler.go:978; bl:internal/provider/connector.go:128 |
| controller-runtime builder and controller options | `For`, `Watches`, `WithEventFilter`, `Named`, `WithOptions`, `Complete`; `MaxConcurrentReconciles`, `RecoverPanic=true`, per-item rate limiter | bl:internal/controller/mytype/wiring.go:88 through :93; bl:internal/controller/config/config.go:48 through :55; rt:pkg/reconciler/customresourcesgate/setup.go:42 through :45; rt:pkg/controller/options.go:87 through :89 |
| controller-runtime predicates and handlers | `DesiredStateChanged` ORs an annotation predicate that ignores the create-pending and create-failed annotations with the label and generation predicates | rt:pkg/resource/predicates.go:55 through :69; used at bl:internal/controller/mytype/wiring.go:91. `EnqueueRequestForProviderConfig` at bl:internal/controller/config/config.go:52 |
| client-go workqueue rate limiters | Global token bucket of `rps` with burst `rps*10`; per-item exponential backoff from 1s to 60s; the `ratelimiter.Reconciler` requeue-after wrapper | rt:pkg/ratelimiter/default.go:32 through :34, rt:pkg/ratelimiter/default.go:42 through :44, rt:pkg/ratelimiter/reconciler.go:54 through :66; `golang.org/x/time/rate` at rt:pkg/ratelimiter/default.go:33 |
| client-go REST config | QPS `rps*5`, burst `rps*10` | rt:pkg/ratelimiter/default.go:48 through :56; called at bl:cmd/provider/main.go:92 |
| client-go event recorder | Annotated Kubernetes events through `record.EventRecorder.AnnotatedEventf` | rt:pkg/event/event.go:105; recorder from `mgr.GetEventRecorderFor` at bl:internal/controller/mytype/wiring.go:55 |
| client-go retry, apimachinery wait and sets | `retry.OnError(retry.DefaultRetry, ...)` in `UpdateCriticalAnnotations`, retrying until the context is canceled (rt:pkg/reconciler/managed/api.go:283); backoff applicator; management-policy sets | rt:pkg/reconciler/managed/api.go:285; rt:pkg/resource/resource.go:241; rt:pkg/reconciler/managed/policies.go:55 through :88 |
| prometheus client_golang (v1.23.2) and controller-runtime metrics registry | MR histograms `managed_resource_first_time_to_reconcile_seconds`, `..._first_time_to_readiness_seconds`, `managed_resource_deletion_seconds`, `managed_resource_drift_seconds` with `kmetrics.ExponentialBuckets(10e-9, 10, 10)`; gauges `managed_resource_exists`, `_ready`, `_synced` | rt:pkg/reconciler/managed/metrics.go:62 through :84; rt:pkg/statemetrics/mr_state_metrics.go:45 through :57; registered at bl:cmd/provider/main.go:120 through :121 |
| grpc (v1.83.2) and protobuf | Change-log client over a unix socket with insecure credentials; `WaitForReady(true)`; 10s default send timeout; `timestamppb` and `structpb` payloads | bl:cmd/provider/main.go:146; rt:pkg/reconciler/managed/changelogger.go:33, :101, :120; rt:pkg/resource/resource.go:479 through :488 |
| evanphx/json-patch (v5.9.11+incompatible) and go-cmp | Reference resolution compares the MR before and after `ResolveReferences` with `cmpopts.EquateEmpty`, builds a JSON merge patch, and sends it as `types.ApplyPatchType` with a field owner and `ForceOwnership`. Secret publishers compare secret data with `cmpopts.EquateEmpty` | rt:pkg/reconciler/managed/api.go:252, :230, :262; rt:pkg/reconciler/managed/api.go:115, :174 |
| spf13/afero | Filesystem credential source | rt:pkg/resource/providerconfig.go:78, rt:pkg/resource/providerconfig.go:101 |
| go-logr, zap, klog | `logging.Logger` wraps logr; zap dev mode under `--debug` | rt:pkg/logging/logging.go:76; bl:cmd/provider/main.go:76 through :87. `SetFilteredKlogLogger` (rt:pkg/logging/klog.go:17) is unused by the baseline |
| apiextensions-apiserver | CRD watch for the safe-start gate; GVKs flip when the CRD is Established | rt:pkg/reconciler/customresourcesgate/reconciler.go:40 through :76, gate calls at :49 and :59; scheme registration at bl:cmd/provider/main.go:115; package capability `safe-start` at bl:package/crossplane.yaml:22 |
| alecthomas/kingpin (v2.4.0) | CLI flags and env vars of the template, not the runtime | bl:cmd/provider/main.go:57 through :74 |


## 6. Scope options

Sizes are raw non-test Go lines from section 1, generated lines in brackets, and heuristic table cases from section 1 and section 3. They measure the Go being matched, not the Rust to be written.

| Option | Go packages | Go lines (generated) | Table cases |
|---|---|---|---|
| A. Minimal: closure only | 17 runtime packages (changelogs proto included), core/v2 | 8861 (1053) | 504 (473 runtime + 31 core/v2) |
| B. Closure plus common extras | A plus `pkg/reference`, `pkg/resource/fake`, `pkg/test` | 10903 (1053) | 602 |
| B+. B plus the optional three | B plus `pkg/webhook`, `pkg/password`, `pkg/certificates` | 11231 (1053) | 622 |
| C. Everything | all 40 packages plus core/v2 | 19668 (2807) | 986 |

Notes that change the cost of each option:

- **Option A still needs test helpers.** The closure's own tests import `pkg/resource/fake` (738 lines) and `pkg/test` (540 lines). Porting the 473 cases needs Rust equivalents of both, even if they stay private. B differs from A in making them public and adding `pkg/reference`.
- **Legacy versus modern is a separate lever.** The baseline is modern only (section 3b). Dropping legacy removes 90 managed-reconciler cases (504 becomes 414 in A, 602 becomes 512 in B). It also removes the legacy policy resolver, the legacy usage tracker, and `APISecretPublisher`, which writes secrets to an arbitrary namespace (rt:pkg/reconciler/managed/api.go:100). The line count of legacy-only code was not measured. Legacy parity has no Go baseline to compare against, because the xp-provider-gen baseline contains only namespaced kinds.
- **The three composition packages are not needed in any option below C.** Their only closure tie is three compile-time assertions (section 1).
- **Behavioral parity has no Go unit tests to lean on.** The baseline has zero `_test.go` files. Its only behavior test is bl:test/behavior/mytype-pause/chainsaw-test.yaml. Parity evidence must come from the runtime's table cases plus end-to-end runs.

**Recommendation (the decision is the user's):** Option B, modern only, with legacy deferred. The baseline MR is namespaced, and pt:'s MR asserts `ModernManaged` (pt:apis/sample/v1alpha1/mytype_types.go:31). `pkg/reference` is needed the moment a provider kind has a cross-resource reference (section 1). pt: imports `pkg/test` in its unit tests (pt:internal/controller/mytype/mytype_test.go:26). `pkg/resource/fake` supplies the mocks the runtime's own reconciler tests drive. Add the optional three only when a provider asks for webhooks or mTLS.
