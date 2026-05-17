import { createRouter, type ErrorComponentProps } from "@tanstack/react-router";

import { RouteErrorBoundary } from "@/components/route-error-boundary";

// Import the generated route tree
import { routeTree } from "./routeTree.gen";

// Create a new router instance
export const getRouter = () => {
  const router = createRouter({
    routeTree,
    context: {},

    scrollRestoration: true,
    defaultPreloadStaleTime: 0,
    defaultErrorComponent: ({ error, reset }: ErrorComponentProps) => (
      <RouteErrorBoundary error={error} reset={reset} />
    ),
  });

  return router;
};
