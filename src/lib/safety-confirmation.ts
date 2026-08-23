import {
  resolveSafetyPolicy,
  type SafetyPolicyInput,
  type StatementClassSummary,
} from "@/lib/safety-policy";

export type SafetyConfirmationConnection = SafetyPolicyInput & {
  name: string;
};

export type SafetyConfirmationRequest = {
  connection: SafetyConfirmationConnection;
  subject:
    | { kind: "statements"; statements: StatementClassSummary[] }
    | { kind: "command"; command: string; destructive: boolean };
};

type PendingConfirmation = SafetyConfirmationRequest & {
  id: string;
  resolve: (confirmed: boolean) => void;
};

const listeners = new Set<() => void>();
const queue: PendingConfirmation[] = [];
let current: PendingConfirmation | null = null;

const emit = () => {
  current = queue[0] ?? null;
  for (const listener of listeners) listener();
};

export const subscribeSafetyConfirmation = (listener: () => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

export const getSafetyConfirmation = () => current;

export const requestSafetyConfirmation = (
  request: SafetyConfirmationRequest,
): Promise<boolean> =>
  new Promise((resolve) => {
    const { name, environment, safeMode, readOnly } = request.connection;
    queue.push({
      ...request,
      connection: { name, environment, safeMode, readOnly },
      id: crypto.randomUUID(),
      resolve,
    });
    emit();
  });

export const resolveSafetyConfirmation = (confirmed: boolean) => {
  const pending = queue.shift();
  if (!pending) return;
  pending.resolve(confirmed);
  emit();
};

export const safetyConfirmationRequiresTyping = (
  request: SafetyConfirmationRequest,
): boolean => {
  const policy = resolveSafetyPolicy(request.connection);
  return (
    (policy.environment === "production" && policy.level === "strict") ||
    (request.subject.kind === "statements" &&
      request.subject.statements.some((statement) => statement.destructive)) ||
    (request.subject.kind === "command" && request.subject.destructive)
  );
};
