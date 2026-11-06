import { describe, test } from "node:test";
import * as assert from "node:assert";
import {
  APP_ROUTE_METADATA,
  HIDDEN_TAB_ROUTE_SCREENS,
  PRIMARY_TAB_ROUTE_NAMES,
  ROUTES,
  TAB_ROUTE_PRIMARY_TAB_BY_SCREEN,
  TAB_ROUTE_NAMES,
  getPrimaryTabRouteNames,
  getPrimaryTabRoutes,
  hrefWithParams,
} from "./routes";

describe("primary tab navigation", () => {
  test("shows only Resources then Calendar while retaining hidden routes", () => {
    const routeNames = [
      TAB_ROUTE_NAMES.profile,
      TAB_ROUTE_NAMES.transferPlanner,
      TAB_ROUTE_NAMES.calendar,
      TAB_ROUTE_NAMES.resources,
    ];

    assert.deepStrictEqual(PRIMARY_TAB_ROUTE_NAMES, [
      TAB_ROUTE_NAMES.resources,
      TAB_ROUTE_NAMES.calendar,
    ]);
    assert.deepStrictEqual(getPrimaryTabRouteNames(routeNames), [
      TAB_ROUTE_NAMES.resources,
      TAB_ROUTE_NAMES.calendar,
    ]);
    assert.deepStrictEqual(
      getPrimaryTabRoutes(routeNames.map((name) => ({ name }))).map((route) => route.name),
      [TAB_ROUTE_NAMES.resources, TAB_ROUTE_NAMES.calendar]
    );
    assert.equal(TAB_ROUTE_PRIMARY_TAB_BY_SCREEN[TAB_ROUTE_NAMES.resources], TAB_ROUTE_NAMES.resources);
    assert.equal(TAB_ROUTE_PRIMARY_TAB_BY_SCREEN[TAB_ROUTE_NAMES.calendar], TAB_ROUTE_NAMES.calendar);
    assert.ok(HIDDEN_TAB_ROUTE_SCREENS.includes(TAB_ROUTE_NAMES.profile));
    assert.ok(HIDDEN_TAB_ROUTE_SCREENS.includes(TAB_ROUTE_NAMES.transferPlanner));
    assert.equal(APP_ROUTE_METADATA.profile.href, "/profile");
    assert.equal(APP_ROUTE_METADATA.transferPlanner.href, "/resources/transfer-planner");
  });

  test("uses Resources as the canonical landing route", () => {
    assert.equal(ROUTES.tabsResources, "/resources");
    assert.equal(APP_ROUTE_METADATA.root.primaryTab, TAB_ROUTE_NAMES.resources);
    assert.equal(APP_ROUTE_METADATA.legacyHome.primaryTab, TAB_ROUTE_NAMES.resources);
  });
});

describe("hrefWithParams", () => {
  test("returns plain string when params is empty", () => {
    assert.strictEqual(hrefWithParams("/path", {}), "/path");
  });

  test("returns plain string when params only has empty strings", () => {
    assert.strictEqual(hrefWithParams("/path", { a: "", b: "   " }), "/path");
  });

  test("returns plain string when params only has empty arrays", () => {
    assert.strictEqual(hrefWithParams("/path", { a: [] }), "/path");
  });

  test("returns plain string when params only has null/undefined", () => {
    assert.strictEqual(hrefWithParams("/path", { a: null, b: undefined }), "/path");
  });

  test("returns object with params when valid params provided", () => {
    assert.deepStrictEqual(hrefWithParams("/path", { a: "1", b: 2, c: true, d: ["x", "y"] }), {
      pathname: "/path",
      params: { a: "1", b: 2, c: true, d: ["x", "y"] },
    });
  });

  test("filters out empty values but keeps valid ones", () => {
    assert.deepStrictEqual(
      hrefWithParams("/path", { a: "1", b: "", c: null, d: undefined, e: [], f: "  ", g: 0 }),
      {
        pathname: "/path",
        params: { a: "1", g: 0 },
      }
    );
  });
});
