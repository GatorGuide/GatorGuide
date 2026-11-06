import { Redirect, type Href } from "expo-router";

import { ROUTES } from "@/constants/routes";

/** Previous-version Home route — send visitors to the Resources landing page. */
export default function TabHomeRedirect() {
  return <Redirect href={ROUTES.tabsResources as Href} />;
}
