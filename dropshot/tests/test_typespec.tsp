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


@route("/authed")
@get
op authed_get(@header authorization: string, @header x_request_id?: string): Widget;

@route("/flexible")
@get
op flexible_get(): Flexible;

@route("/instance-state")
@get
op instance_state_get(): InstanceState;

@doc("Old endpoint")
#deprecated "deprecated"
@route("/old")
@get
op old_get(): void;

@doc("Get paint job")
@route("/paint")
@get
op paint_get(): PaintJob;

@doc("Health check")
@route("/ping")
@get
op ping(): void;

@doc("List projects")
@route("/projects")
@get
op project_list(
  @query limit?: uint32 | null,
  @query name_or_id?: string | null,
  @query page_token?: string | null
): ResultsPage<Widget>;

@doc("Raw bytes")
@route("/raw")
@get
op raw_get(): void;

@doc("Create a widget")
@route("/widgets")
@post
op widget_create(@body body: WidgetCreate): {
  @statusCode _: 201;
  @body body: Widget;
};

@doc("Delete a widget")
@tag("widgets")
@route("/widgets/{id}")
@delete
op widget_delete(@path id: string): void;

@doc("Get a widget")
@route("/widgets/{id}")
@get
op widget_get(@path id: string): Widget;

@route("/widgets/{id}")
@put
op widget_update(
  @path id: string,
  @query force?: boolean | null,
  @body body: WidgetCreate
): void;

@doc("Get with etag")
@route("/with-etag")
@get
op with_etag(): {
  @header etag: string;
  @body body: Widget;
};
