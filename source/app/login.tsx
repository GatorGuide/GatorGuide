import { Redirect } from "expo-router";
import { ROUTES } from "@/constants/routes";

export default function Login() {
  return <Redirect href={ROUTES.tabsResources} />;
}
