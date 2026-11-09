import { Redirect, useLocalSearchParams } from "expo-router";
import AuthPage from "../components/pages/AuthPage";
import { ROUTES } from "@/constants/routes";

export default function Login() {
  const { intent } = useLocalSearchParams<{ intent?: string }>();
  if (intent !== "account") {
    return <Redirect href={ROUTES.tabsResources} />;
  }
  return <AuthPage />;
}
