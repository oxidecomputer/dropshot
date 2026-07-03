import "@typespec/http";
import "@typespec/openapi";

using Http;
using OpenAPI;

@service(#{ title: "Widget Service" })
@info(#{ version: "1.0.0", contact: #{ name: "Widget Co", url: "https://widgets.example.com", email: "api@widgets.example.com" } })
@doc("API for managing widgets and their component parts")
namespace WidgetService;

model ResultsPage<T> {
  @doc("list of items on this page of results")
  items: T[];
  @doc("token used to fetch the next page of results (if any)")
  next_page?: string | null;
}

@format("uuid")
scalar uuid extends string;

@doc("Response with untyped body")
@defaultResponse
model FreeformResponse {
  @body body: bytes;
}

enum Color {
  Red,
  Green,
  Blue
}

model CompositeCreate {
  @doc("Desired color (if any)")
  color?: Color | null;
  @doc("The widget's name")
  name: string;
  @doc("Preferred color for new composites")
  preferred_color?: Color | null = null;
  @doc("Desired shape (if any)")
  shape?: Shape | null;
  @doc("Whether to start immediately")
  start?: boolean = true;
  @doc("Initial set of widgets")
  widgets?: Widget[] = #[];
}

model Constrained {
  @doc("Must be between 1 and 100")
  @minValue(1)
  @maxValue(100)
  count: uint32;
  @doc("Between 1 and 10 items")
  @minItems(1)
  @maxItems(10)
  items: string[];
  @doc("Short name")
  @minLength(1)
  @maxLength(63)
  name: string;
  @doc("DNS-style name")
  @pattern("^[a-z][a-z0-9-]*$")
  slug: string;
}

model DeviceAuthRequest {
  client_id: string;
}

model Disk {
  name: string;
}

@discriminated(#{envelope: "none", discriminatorPropertyName: "type"})
union DiskSource {
  @doc("Create a disk from a disk snapshot")
  snapshot: {
    type: "snapshot",
    @doc("If true, the disk will be read-only.")
    read_only?: boolean = false,
    snapshot_id: uuid,
  },
  @doc("Create a blank disk")
  blank: {
    type: "blank",
    @minValue(0)
    block_size: uint32,
  },
}

@discriminated(#{envelope: "none", discriminatorPropertyName: "state"})
union DiskState {
  @doc("Disk is being initialized")
  creating: {
    state: "creating",
  },
  @doc("Disk is detached")
  detached: {
    state: "detached",
  },
  @doc("Disk is being attached to an instance")
  attaching: {
    state: "attaching",
    @doc("The instance this disk is being attached to")
    instance: uuid,
  },
  @doc("Disk is attached to an instance")
  attached: {
    state: "attached",
    @doc("The instance this disk is attached to")
    instance: uuid,
  },
  @doc("Disk has been destroyed")
  destroyed: {
    state: "destroyed",
  },
}

@doc("Error information from a response.")
@error
model Error {
  @minValue(400)
  @maxValue(599)
  @statusCode statusCode: int32;
  error_code?: string;
  message: string;
  request_id: string;
}

model Flexible {
  @doc("Free-form metadata")
  metadata: unknown;
  @doc("Required string")
  name: string;
  @doc("Nullable field")
  nickname?: string | null;
  @doc("A list of tags")
  tags: string[];
}

enum InstanceState {
  @doc("The instance is running")
  running,
  @doc("The instance is stopped")
  stopped,
  @doc("The instance is being created")
  creating,
  @doc("The instance is being destroyed")
  destroying
}

model InterfaceNumUnknown {
  `unknown`: uint32;
}

model InterfaceNumIfIndex {
  if_index: uint32;
}

model InterfaceNumPortNumber {
  port_number: uint32;
}

union InterfaceNum {
  InterfaceNumUnknown,
  InterfaceNumIfIndex,
  InterfaceNumPortNumber,
}

model LookupResult {
  id: uuid;
  target: NameOrId;
}

model MeasurementBatch {
  @doc("Measured values; null entries are missing samples.")
  values: (int64 | null)[];
}

@doc("A name unique within the parent collection")
@minLength(1)
@maxLength(63)
@pattern("^[a-z]([a-zA-Z0-9-]*[a-zA-Z0-9]+)?$")
scalar Name extends string;

union NameOrId {
  uuid,
  Name,
}

model NetworkInfo {
  interface_num: InterfaceNum;
}

model PaintJob {
  color: Color;
  shape: Shape;
}

model PhysicalDisk {
  model_number: string;
  name: string;
}

@discriminated(#{envelope: "none", discriminatorPropertyName: "kind"})
union Shape {
  Circle: {
    kind: "Circle",
    radius: float64,
  },
  Rectangle: {
    kind: "Rectangle",
    height: float64,
    width: float64,
  },
}

model Widget {
  @doc("How many we have")
  @minValue(0)
  count: uint32;
  @doc("Is it enabled?")
  enabled: boolean;
  @doc("The widget's name")
  name: string;
}

model WidgetCreate {
  @doc("Optional description")
  description?: string | null;
  @doc("The widget's name")
  name: string;
}

model WithDefaults {
  @doc("Answer to everything")
  @minValue(0)
  answer?: uint32 = 42;
  @doc("Is it on?")
  enabled?: boolean = false;
  @doc("Optional name")
  label?: string = "unnamed";
  @doc("Tags with default empty list")
  tags?: string[] = #[];
}


@route("/authed")
@get
op authed_get(@header authorization: string, @header x_request_id?: string): Widget | Error;

@summary("Create a composite")
@route("/composites")
@post
op composite_create(@body body: CompositeCreate): {
  @statusCode _: 201;
  @body body: Widget;
} | Error;

@route("/constrained")
@get
op constrained_get(): Constrained | Error;

@summary("Start device auth")
@route("/device/auth")
@post
op device_auth(@header contentType: "application/x-www-form-urlencoded", @body body: DeviceAuthRequest): void | Error;

@summary("Get device token")
@route("/device/token")
@post
op device_token(): FreeformResponse | Error;

@summary("Get disk source")
@route("/disk-source")
@get
op disk_source_get(): DiskSource | Error;

@summary("List disks")
@doc("Returns a paginated list of all disks the caller can see. Use the `limit` and `page_token` query parameters to iterate through results.")
@extension("x-dropshot-pagination", #{required: #[]})
@route("/disks")
@get
op disk_list(
  @doc("Maximum number of items returned by a single call")
  @minValue(1) @query limit?: uint32 | null,
  @query name?: string | null,
  @doc("Token returned by previous call to retrieve the subsequent page")
  @query page_token?: string | null
): ResultsPage<Disk> | Error;

@summary("Get disk state")
@route("/disks/{id}/state")
@get
op disk_state_get(@path id: string): DiskState | Error;

@route("/flexible")
@get
op flexible_get(): Flexible | Error;

@route("/instance-state")
@get
op instance_state_get(): InstanceState | Error;

@summary("Redirect to login")
@route("/login")
@get
op login_redirect(): {
  @statusCode _: 303;
  @header location: string;
} | Error;

@route("/lookup")
@get
op lookup(): LookupResult | Error;

@summary("Get measurements")
@route("/measurements")
@get
op measurement_get(): MeasurementBatch | Error;

@summary("Get network info")
@route("/network-info")
@get
op network_info_get(): NetworkInfo | Error;

@summary("Old endpoint")
#deprecated "deprecated"
@route("/old")
@get
op old_get(): void | Error;

@summary("Get paint job")
@route("/paint")
@get
op paint_get(): PaintJob | Error;

@summary("List physical disks")
@extension("x-dropshot-pagination", #{required: #[]})
@route("/physical-disks")
@get
op physical_disk_list(
  @doc("Maximum number of items returned by a single call")
  @minValue(1) @query limit?: uint32 | null,
  @query name?: string | null,
  @doc("Token returned by previous call to retrieve the subsequent page")
  @query page_token?: string | null
): ResultsPage<PhysicalDisk> | Error;

@summary("Health check")
@route("/ping")
@get
op ping(): void | Error;

@summary("List projects")
@extension("x-dropshot-pagination", #{required: #[]})
@route("/projects")
@get
op project_list(
  @doc("Maximum number of items returned by a single call")
  @minValue(1) @query limit?: uint32 | null,
  @query name_or_id?: string | null,
  @doc("Token returned by previous call to retrieve the subsequent page")
  @query page_token?: string | null
): ResultsPage<Widget> | Error;

@summary("Raw bytes")
@route("/raw")
@get
op raw_get(): void | Error;

@summary("Start a task")
@route("/tasks")
@post
op task_start(@body body: WidgetCreate): {
  @statusCode _: 202;
  @body body: Widget;
} | Error;

@summary("Upload raw bytes")
@route("/upload")
@post
op upload_bytes(@header contentType: "application/octet-stream", @body body: bytes): void | Error;

@summary("Upload multipart")
@route("/upload-multipart")
@post
op upload_multipart(@header contentType: "multipart/form-data", @body body: bytes): void | Error;

@summary("Create a widget")
@route("/widgets")
@post
op widget_create(@body body: WidgetCreate): {
  @statusCode _: 201;
  @body body: Widget;
} | Error;

@summary("Delete a widget")
@tag("widgets")
@route("/widgets/{id}")
@delete
op widget_delete(@path id: string): void | Error;

@summary("Get a widget")
@route("/widgets/{id}")
@get
op widget_get(@path id: string): Widget | Error;

@route("/widgets/{id}")
@put
op widget_update(
  @path id: string,
  @doc("Force the update?")
  @query force?: boolean | null,
  @body body: WidgetCreate
): void | Error;

@route("/with-defaults")
@post
op with_defaults_create(@body body: WithDefaults): {
  @statusCode _: 201;
  @body body: WithDefaults;
} | Error;

@summary("Get with etag")
@route("/with-etag")
@get
op with_etag(): {
  @header etag: string;
  @body body: Widget;
} | Error;
