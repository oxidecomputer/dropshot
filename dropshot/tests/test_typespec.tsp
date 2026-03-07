import "@typespec/http";
import "@typespec/openapi";

using Http;
using OpenAPI;

@service(#{ title: "Widget Service" })
@info(#{ version: "1.0.0" })
namespace WidgetService;

model ResultsPage<T> {
  @doc("list of items on this page of results")
  items: T[];
  @doc("token used to fetch the next page of results (if any)")
  next_page?: string | null;
}

enum Color {
  Red,
  Green,
  Blue
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

model PaintJob {
  color: Color;
  shape: Shape;
}

@discriminator("kind")
model Shape {
  kind: string;
}

model Circle extends Shape {
  kind: "Circle";
  radius: float64;
}

model Rectangle extends Shape {
  kind: "Rectangle";
  height: float64;
  width: float64;
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

@route("/constrained")
@get
op constrained_get(): Constrained | Error;

@route("/flexible")
@get
op flexible_get(): Flexible | Error;

@route("/instance-state")
@get
op instance_state_get(): InstanceState | Error;

@doc("Old endpoint")
#deprecated "deprecated"
@route("/old")
@get
op old_get(): void | Error;

@doc("Get paint job")
@route("/paint")
@get
op paint_get(): PaintJob | Error;

@doc("Health check")
@route("/ping")
@get
op ping(): void | Error;

@doc("List projects")
@route("/projects")
@get
op project_list(
  @query limit?: uint32 | null,
  @query name_or_id?: string | null,
  @query page_token?: string | null
): ResultsPage<Widget> | Error;

@doc("Raw bytes")
@route("/raw")
@get
op raw_get(): void | Error;

@doc("Create a widget")
@route("/widgets")
@post
op widget_create(@body body: WidgetCreate): {
  @statusCode _: 201;
  @body body: Widget;
} | Error;

@doc("Delete a widget")
@tag("widgets")
@route("/widgets/{id}")
@delete
op widget_delete(@path id: string): void | Error;

@doc("Get a widget")
@route("/widgets/{id}")
@get
op widget_get(@path id: string): Widget | Error;

@route("/widgets/{id}")
@put
op widget_update(
  @path id: string,
  @query force?: boolean | null,
  @body body: WidgetCreate
): void | Error;

@route("/with-defaults")
@post
op with_defaults_create(@body body: WithDefaults): {
  @statusCode _: 201;
  @body body: WithDefaults;
} | Error;

@doc("Get with etag")
@route("/with-etag")
@get
op with_etag(): {
  @header etag: string;
  @body body: Widget;
} | Error;
