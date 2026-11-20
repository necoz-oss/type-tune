import type { CliOptionSection } from "../../entities/cli-option/model.js";

export const cliOptionSections: CliOptionSection[] = [
  {
    id: "shared-options",
    title: "Shared scan and plan options",
    options: [
      {
        command: "shared",
        name: "--project",
        value: "<path>",
        defaultValue: "tsconfig.json",
        description: "Path to the TypeScript project config.",
      },
      {
        command: "shared",
        name: "--min-referrers",
        value: "<number>",
        defaultValue: "2",
        description: "Minimum unique referrer files required for a candidate.",
      },
      {
        command: "shared",
        name: "--referrer-count-exclude",
        value: "<globs>",
        defaultValue: "none",
        description:
          "Comma-separated project-relative *, **, and ? globs excluded from eligibility counts but still rewritten.",
      },
      {
        command: "shared",
        name: "--placement-style",
        value: "<style>",
        defaultValue: "collocation",
        description: "Destination strategy: collocation or repo-types.",
      },
      {
        command: "shared",
        name: "--include",
        value: "<kinds>",
        defaultValue: "type,interface",
        description: "Declaration kinds included in analysis.",
      },
      {
        command: "shared",
        name: "--ignore-file",
        value: "<path>",
        defaultValue: "none",
        description: "Candidate IDs, stable keys, or sourcePath#SymbolName entries to ignore.",
      },
      {
        command: "shared",
        name: "--include-existing-types",
        defaultValue: "false",
        description: "Include declarations in types directories or types.ts files.",
      },
      {
        command: "shared",
        name: "--include-generated",
        defaultValue: "false",
        description: "Include generated source files in analysis.",
      },
      {
        command: "shared",
        name: "--compiler-resolution-bridge",
        defaultValue: "false",
        description:
          "Experimental: ask the project-local TypeScript compiler for resolution evidence. This is not the default scan path.",
      },
    ],
  },
  {
    id: "scan-options",
    title: "scan options",
    options: [
      {
        command: "scan",
        name: "--format",
        value: "<format>",
        defaultValue: "text",
        description: "Output format: text or json.",
      },
      {
        command: "scan",
        name: "--verbose",
        defaultValue: "false",
        description: "Include referrer and import-strategy details.",
      },
      {
        command: "scan",
        name: "--explain",
        defaultValue: "false",
        description: "Explain why exported declarations were omitted.",
      },
    ],
  },
  {
    id: "plan-options",
    title: "plan options",
    description:
      "plan writes .type-tune/plan.json and .type-tune/plan.md. Edit approval and destinations in the Markdown file.",
    options: [
      {
        command: "plan",
        name: "--out",
        value: "<path>",
        defaultValue: ".type-tune/plan.json",
        description: "Path for structured plan JSON.",
      },
      {
        command: "plan",
        name: "--markdown",
        value: "<path>",
        defaultValue: ".type-tune/plan.md",
        description: "Path for human-reviewable Markdown.",
      },
      {
        command: "plan",
        name: "--default-status",
        value: "<status>",
        defaultValue: "ignore",
        description: "Initial entry status: apply or ignore.",
      },
    ],
  },
  {
    id: "review-options",
    title: "review options",
    description: "Interactively approve or ignore candidates in plan.md.",
    options: [
      {
        command: "review",
        name: "--markdown",
        value: "<path>",
        defaultValue: "required",
        description: "Path to the plan Markdown file to review and update.",
      },
    ],
  },
  {
    id: "apply-options",
    title: "apply options",
    options: [
      {
        command: "apply",
        name: "--plan",
        value: "<path>",
        defaultValue: "required",
        description: "Path to plan.json.",
      },
      {
        command: "apply",
        name: "--markdown",
        value: "<path>",
        defaultValue: "required",
        description:
          "Reviewed plan.md. Required unless --accept-unreviewed is set. Supplies apply/ignore status and any edited destination.",
      },
      {
        command: "apply",
        name: "--dry-run",
        defaultValue: "false",
        description: "Preview and validate changes, then roll back file writes.",
      },
      {
        command: "apply",
        name: "--accept-unreviewed",
        defaultValue: "false",
        description: "Use plan.json statuses without reviewed Markdown.",
      },
      {
        command: "apply",
        name: "--compiler-resolution-bridge",
        defaultValue: "false",
        description:
          "Experimental: re-check compiler resolution evidence before apply or dry-run writes.",
      },
    ],
  },
  {
    id: "extract-options",
    title: "extract options",
    options: [
      {
        command: "extract",
        name: "<symbol>",
        defaultValue: "required",
        description: "Symbol name to extract.",
      },
      {
        command: "extract",
        name: "--from",
        value: "<path>",
        defaultValue: "required",
        description: "Source file containing the symbol.",
      },
      {
        command: "extract",
        name: "--to",
        value: "<path>",
        defaultValue: "derived",
        description: "Explicit destination file path.",
      },
      {
        command: "extract",
        name: "--project",
        value: "<path>",
        defaultValue: "tsconfig.json",
        description: "Path to the TypeScript project config.",
      },
      {
        command: "extract",
        name: "--placement-style",
        value: "<style>",
        defaultValue: "collocation",
        description: "Destination strategy when --to is omitted.",
      },
      {
        command: "extract",
        name: "--dry-run",
        defaultValue: "false",
        description: "Preview and validate changes, then roll back file writes.",
      },
      {
        command: "extract",
        name: "--include-generated",
        defaultValue: "false",
        description: "Include generated source files.",
      },
      {
        command: "extract",
        name: "--compiler-resolution-bridge",
        defaultValue: "false",
        description:
          "Experimental: require compiler resolution evidence before extract or dry-run writes.",
      },
    ],
  },
  {
    id: "ai-review-options",
    title: "AI review options",
    description:
      "Optional plan annotations only. AI is not part of the deterministic rewrite path.",
    options: [
      {
        command: "ai-review",
        name: "--ai-review",
        defaultValue: "false",
        description: "Prepare AI review for the generated plan.",
      },
      {
        command: "ai-review",
        name: "--ai-dry-run",
        defaultValue: "false",
        description: "Write the request without calling a provider.",
      },
      {
        command: "ai-review",
        name: "--ai-provider",
        value: "<provider>",
        defaultValue: "mock",
        description:
          "Provider: mock, openai, openai-compatible, anthropic, or ollama. mock reads a local response file and does not call a network API.",
      },
      {
        command: "ai-review",
        name: "--ai-context",
        value: "<mode>",
        defaultValue: "snippets",
        description: "Context mode: summary, snippets, or full.",
      },
      {
        command: "ai-review",
        name: "--ai-request-out",
        value: "<path>",
        defaultValue: "none",
        description: "Path for AI request JSON.",
      },
      {
        command: "ai-review",
        name: "--ai-response-file",
        value: "<path>",
        defaultValue: "required for mock",
        description: "Local JSON decisions for the mock provider. Required when mock actually runs.",
      },
      {
        command: "ai-review",
        name: "--ai-endpoint",
        value: "<url>",
        defaultValue: "provider default",
        description:
          "Required for openai-compatible. OpenAI, Anthropic, and Ollama use their own defaults when omitted.",
      },
      {
        command: "ai-review",
        name: "--ai-model",
        value: "<model>",
        defaultValue: "required except mock",
        description: "Required for openai, openai-compatible, anthropic, and ollama.",
      },
      {
        command: "ai-review",
        name: "--ai-api-key-env",
        value: "<name>",
        defaultValue: "provider default",
        description: "Environment variable containing the API key.",
      },
      {
        command: "ai-review",
        name: "--ai-timeout-ms",
        value: "<number>",
        defaultValue: "60000",
        description: "Request timeout in milliseconds.",
      },
      {
        command: "ai-review",
        name: "--ai-on-error",
        value: "<mode>",
        defaultValue: "fail",
        description: "Failure mode: fail, skip, or mark-needs-review.",
      },
      {
        command: "ai-review",
        name: "--ai-cache",
        value: "<path>",
        defaultValue: "none",
        description: "AI review cache JSON path.",
      },
      {
        command: "ai-review",
        name: "--ai-max-candidates",
        value: "<number>",
        defaultValue: "none",
        description: "Maximum candidates sent for review.",
      },
      {
        command: "ai-review",
        name: "--ai-max-context-bytes",
        value: "<number>",
        defaultValue: "none",
        description: "Maximum source context bytes sent for review.",
      },
    ],
  },
];
