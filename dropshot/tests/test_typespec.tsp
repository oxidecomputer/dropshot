import "@typespec/http";
import "@typespec/openapi";

using Http;
using OpenAPI;

@service(#{ title: "Widget Service" })
@info(#{ version: "1.0.0" })
namespace WidgetService;

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
  description?: string;
  @doc("The widget's name")
  name: string;
}


@doc("Health check")
@route("/ping")
@get
op ping(): void;

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
  @query force?: boolean,
  @body body: WidgetCreate
): void;
