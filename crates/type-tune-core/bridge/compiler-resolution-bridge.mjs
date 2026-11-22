// Experimental TypeScript resolution bridge.
// This process loads the project-local TypeScript package. That package is trusted code:
// a hostile compiler can bypass this host. The host only constrains compiler APIs that
// read through the supplied policy. It does not sandbox arbitrary compiler code.

import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const PROTOCOL = "type-tune.compiler-resolution.v1";
const require = createRequire(import.meta.url);

const state = {
  aborted: null,
  unauthorized: [],
  unsupported: [],
  contentReads: new Set(),
  resolutions: [],
  peakRss: 0,
};

let emitted = false;

function emit(payload) {
  if (emitted) {
    return;
  }
  emitted = true;
  state.peakRss = Math.max(state.peakRss, process.memoryUsage().rss);
  payload.peakRssBytes = state.peakRss;
  process.stdout.write(`${JSON.stringify(payload)}\n`);
}

function emptyLists() {
  return {
    sources: [],
    edges: [],
    diagnostics: [],
    resolutionFailures: [],
    unauthorized: [],
    contentReads: [],
  };
}

function basePayload(status, complete, coverage, message, extra = {}) {
  return {
    protocol: PROTOCOL,
    policyVersion: 1,
    status,
    complete,
    coverage,
    message,
    compilerVersion: extra.compilerVersion ?? null,
    moduleResolution: extra.moduleResolution ?? null,
    timingsMs: extra.timingsMs ?? { config: 0, program: 0, graph: 0 },
    ...emptyLists(),
    ...extra,
    contentReads: [...state.contentReads],
  };
}

function marker(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}

function isInside(root, candidate) {
  const relative = path.relative(root, candidate);
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative));
}

function isVirtual(fileName) {
  return fileName.includes("\0") || fileName.includes("?") || fileName.includes("\u0000") || fileName.startsWith("virtual:");
}

function classify(fileName, policy) {
  const base = path.basename(fileName);
  if (policy.declarationSuffixes.some((suffix) => base.endsWith(suffix))) {
    return "declaration";
  }
  if (policy.implementationExtensions.some((suffix) => base.endsWith(suffix))) {
    return "implementation";
  }
  if (policy.metadataBasenames.includes(base) || (base.startsWith("tsconfig.") && base.endsWith(".json"))) {
    return "metadata";
  }
  return "other";
}

function hasSymlinkComponent(root, candidate) {
  const relative = path.relative(root, candidate);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    return false;
  }
  let current = root;
  for (const part of relative.split(path.sep)) {
    if (!part || part === ".") {
      continue;
    }
    current = path.join(current, part);
    let stat;
    try {
      stat = fs.lstatSync(current);
    } catch {
      return false;
    }
    if (stat.isSymbolicLink()) {
      return true;
    }
  }
  return false;
}

function canonicalPath(fileName) {
  try {
    return fs.realpathSync.native(fileName);
  } catch {
    return null;
  }
}

function remember(list, record) {
  const key = JSON.stringify(record);
  if (!list.some((item) => JSON.stringify(item) === key)) {
    list.push(record);
  }
}

function authorize(fileName, access, policy) {
  const logicalPath = path.resolve(fileName);
  if (isVirtual(logicalPath)) {
    return {
      action: "unsupported",
      record: {
        logicalPath,
        canonicalPath: logicalPath,
        reason: policy.reasons.virtualIdentity,
      },
    };
  }
  const real = canonicalPath(logicalPath);
  if ((real && isInside(policy.typescriptLibRoot, real)) || isInside(policy.typescriptLibRoot, logicalPath)) {
    return { action: "allow", role: "library" };
  }
  const role = classify(logicalPath, policy);
  if (role !== "implementation") {
    return { action: "allow", role };
  }
  if (!real) {
    if (access === "exists") {
      return { action: "allow", role, missing: true };
    }
    return {
      action: "unsupported",
      record: {
        logicalPath,
        canonicalPath: logicalPath,
        reason: policy.reasons.unresolvedIdentity,
      },
    };
  }
  const extension = policy.unsupportedImplementationExtensions.find((item) => path.basename(logicalPath).endsWith(item));
  if (extension) {
    return {
      action: "unsupported",
      record: {
        logicalPath,
        canonicalPath: real,
        reason: policy.reasons.frameworkIdentity,
      },
    };
  }
  if (!isInside(policy.projectRoot, real)) {
    return {
      action: "deny",
      record: {
        logicalPath,
        canonicalPath: real,
        reason: policy.reasons.outsideBoundary,
      },
    };
  }
  const segments = logicalPath.split(path.sep);
  if (segments.includes(policy.dependencyPathSegment) || real.split(path.sep).includes(policy.dependencyPathSegment)) {
    return {
      action: "deny",
      record: {
        logicalPath,
        canonicalPath: real,
        reason: policy.reasons.dependencyImplementation,
      },
    };
  }
  if (!isInside(policy.projectRoot, logicalPath) || hasSymlinkComponent(policy.projectRoot, logicalPath)) {
    return {
      action: "unsupported",
      record: {
        logicalPath,
        canonicalPath: real,
        reason: policy.reasons.symlinkIdentity,
      },
    };
  }
  return { action: "allow", role, logicalPath, canonicalPath: real };
}

function reject(decision) {
  if (decision.action === "deny") {
    remember(state.unauthorized, decision.record);
    state.aborted = "unauthorized";
    throw marker("TYPE_TUNE_UNAUTHORIZED", decision.record.reason);
  }
  if (decision.action === "unsupported") {
    remember(state.unsupported, decision.record);
    state.aborted = "unsupported";
    throw marker("TYPE_TUNE_UNSUPPORTED", decision.record.reason);
  }
}

function guardedReadFile(originalReadFile, policy) {
  return (fileName) => {
    const decision = authorize(fileName, "content", policy);
    reject(decision);
    const text = originalReadFile(fileName);
    if (typeof text === "string") {
      state.contentReads.add(path.resolve(fileName));
    }
    state.peakRss = Math.max(state.peakRss, process.memoryUsage().rss);
    return text;
  };
}

function diagnosticOf(diagnostic, typescript) {
  return {
    code: diagnostic.code ?? 0,
    category: typescript.DiagnosticCategory[diagnostic.category] ?? "unknown",
    message: typescript.flattenDiagnosticMessageText(diagnostic.messageText, "\n"),
  };
}

function resolutionModeName(typescript, options) {
  const kind = options.moduleResolution;
  if (kind === undefined) {
    return "default";
  }
  const name = Object.entries(typescript.ModuleResolutionKind).find(([key, value]) => value === kind && Number.isNaN(Number(key)));
  return name ? name[0] : null;
}

function moduleSpecifier(node, typescript) {
  if (!node) {
    return null;
  }
  if (typescript.isStringLiteralLike?.(node)) {
    return node.text;
  }
  if (typescript.isStringLiteral(node)) {
    return node.text;
  }
  return null;
}

function importTypeText(node, typescript) {
  const argument = node.argument ?? node.argumentType;
  if (!argument) {
    return null;
  }
  if (typescript.isLiteralTypeNode(argument)) {
    return moduleSpecifier(argument.literal, typescript);
  }
  return null;
}

function collectSpecifiers(sourceFile, typescript) {
  const specs = [];
  const visit = (node) => {
    if (typescript.isImportDeclaration(node) || typescript.isExportDeclaration(node)) {
      const specifier = moduleSpecifier(node.moduleSpecifier, typescript);
      if (specifier) {
        specs.push({ specifier, kind: typescript.isExportDeclaration(node) ? "export" : "import" });
      }
    } else if (typescript.isImportTypeNode?.(node)) {
      const specifier = importTypeText(node, typescript);
      if (specifier) {
        specs.push({ specifier, kind: "import-type" });
      }
    } else if (typescript.isCallExpression(node) && node.expression.kind === typescript.SyntaxKind.ImportKeyword) {
      const specifier = moduleSpecifier(node.arguments?.[0], typescript);
      if (specifier) {
        specs.push({ specifier, kind: "dynamic-import" });
      }
    } else if (typescript.isImportEqualsDeclaration(node) && typescript.isExternalModuleReference?.(node.moduleReference)) {
      const specifier = moduleSpecifier(node.moduleReference.expression, typescript);
      if (specifier) {
        specs.push({ specifier, kind: "import-equals" });
      }
    }
    typescript.forEachChild(node, visit);
  };
  visit(sourceFile);
  for (const reference of sourceFile.referencedFiles ?? []) {
    specs.push({ specifier: reference.fileName, kind: "triple-slash", reference: true });
  }
  for (const reference of sourceFile.typeReferenceDirectives ?? []) {
    specs.push({ specifier: reference.fileName, kind: "type-reference", typeReference: true });
  }
  return specs;
}

function resolvedFile(resolution) {
  return resolution?.resolvedModule?.resolvedFileName ?? resolution?.resolvedTypeReferenceDirective?.resolvedFileName ?? null;
}

function run(request) {
  if (request.protocol !== PROTOCOL || request.policy?.version !== 1) {
    return basePayload("malformed", false, "rejected", "Bridge request protocol or policy version is not supported.");
  }
  const policy = {
    ...request.policy,
    projectRoot: path.resolve(request.policy.projectRoot),
    typescriptLibRoot: path.resolve(request.policy.typescriptLibRoot),
  };
  const started = Date.now();
  const typescript = require(request.typescriptEntry);
  const version = typescript.version ?? "0.0.0";
  const [major, minor] = version.split(".").map((part) => Number(part));
  if (!Number.isInteger(major) || major < 4 || (major === 4 && minor < 9) || major > 6) {
    return basePayload("unsupported", false, "rejected", `TypeScript ${version} is outside the tested 4.9–6.x compiler range.`, {
      compilerVersion: version,
    });
  }
  const originalReadFile = typescript.sys.readFile.bind(typescript.sys);
  const readFile = guardedReadFile(originalReadFile, policy);
  typescript.sys.readFile = readFile;
  typescript.sys.writeFile = () => {
    state.aborted = "unsupported";
    remember(state.unsupported, { logicalPath: "", canonicalPath: "", reason: policy.reasons.emitRefused });
    throw marker("TYPE_TUNE_NO_EMIT", policy.reasons.emitRefused);
  };
  const host = typescript.createCompilerHost(request.compilerOptionsHint ?? {}, true);
  host.readFile = readFile;
  host.writeFile = () => {
    state.aborted = "unsupported";
    throw marker("TYPE_TUNE_NO_EMIT", policy.reasons.emitRefused);
  };
  host.fileExists = (fileName) => {
    const decision = authorize(fileName, "exists", policy);
    // Existence probes must not reject a declaration package that also contains
    // an unread implementation file. Content reads and resolved targets decide.
    if (decision.action === "unsupported") {
      reject(decision);
    }
    return typescript.sys.fileExists(fileName);
  };
  const originalGetSourceFile = host.getSourceFile.bind(host);
  host.getSourceFile = (fileName, languageVersion, onError, shouldCreateNewSourceFile) => {
    const decision = authorize(fileName, "content", policy);
    reject(decision);
    return originalGetSourceFile(fileName, languageVersion, onError, shouldCreateNewSourceFile);
  };
  if (host.getSourceFileByPath) {
    const originalGetSourceFileByPath = host.getSourceFileByPath.bind(host);
    host.getSourceFileByPath = (fileName, filePath, languageVersion, onError, shouldCreateNewSourceFile) => {
      reject(authorize(fileName, "content", policy));
      return originalGetSourceFileByPath(fileName, filePath, languageVersion, onError, shouldCreateNewSourceFile);
    };
  }
  const configRead = typescript.readConfigFile(request.projectPath, readFile);
  if (configRead.error) {
    return basePayload("diagnostics", false, "rejected", "TypeScript configuration could not be read.", {
      compilerVersion: version,
      diagnostics: [diagnosticOf(configRead.error, typescript)],
      unauthorized: state.unauthorized,
      timingsMs: { config: Date.now() - started, program: 0, graph: 0 },
    });
  }
  const configDirectory = path.dirname(path.resolve(request.projectPath));
  const parsed = typescript.parseJsonConfigFileContent(configRead.config, host, configDirectory, undefined, path.resolve(request.projectPath));
  const configErrors = (parsed.errors ?? []).filter((diagnostic) => diagnostic.category === typescript.DiagnosticCategory.Error);
  const modeName = resolutionModeName(typescript, parsed.options ?? {});
  if (configErrors.length > 0 || modeName === null) {
    return basePayload("diagnostics", false, "rejected", "TypeScript configuration is invalid or requests an unsupported resolution mode.", {
      compilerVersion: version,
      moduleResolution: modeName,
      diagnostics: configErrors.map((diagnostic) => diagnosticOf(diagnostic, typescript)),
      unauthorized: state.unauthorized,
      timingsMs: { config: Date.now() - started, program: 0, graph: 0 },
    });
  }
  if (Array.isArray(parsed.projectReferences) && parsed.projectReferences.length > 0) {
    return basePayload("unsupported", false, "rejected", policy.reasons.projectReferences, {
      compilerVersion: version,
      moduleResolution: modeName,
      timingsMs: { config: Date.now() - started, program: 0, graph: 0 },
    });
  }
  if (Array.isArray(parsed.options.plugins) && parsed.options.plugins.length > 0) {
    return basePayload("unsupported", false, "rejected", policy.reasons.plugins, {
      compilerVersion: version,
      moduleResolution: modeName,
      timingsMs: { config: Date.now() - started, program: 0, graph: 0 },
    });
  }
  if ((parsed.fileNames ?? []).some((fileName) => isVirtual(fileName) || classify(fileName, policy) === "implementation" && policy.unsupportedImplementationExtensions.some((suffix) => path.basename(fileName).endsWith(suffix)))) {
    return basePayload("unsupported", false, "rejected", policy.reasons.frameworkIdentity, {
      compilerVersion: version,
      moduleResolution: modeName,
      timingsMs: { config: Date.now() - started, program: 0, graph: 0 },
    });
  }
  const options = { ...parsed.options, noEmit: true, incremental: false, composite: false, declaration: false, declarationMap: false, emitDeclarationOnly: false };
  delete options.tsBuildInfoFile;
  const configMs = Date.now() - started;
  const resolutionHost = { ...host, resolveModuleNames: undefined, resolveModuleNameLiterals: undefined };
  const noteResolvedTarget = (specifier, containingFile, resolvedName) => {
    if (!resolvedName) {
      return;
    }
    const decision = authorize(resolvedName, "exists", policy);
    if (decision.action === "deny") {
      remember(state.unauthorized, { ...decision.record, from: path.resolve(containingFile), specifier });
    }
    if (decision.action === "unsupported") {
      reject(decision);
    }
  };
  const resolveOne = (specifier, containingFile, resolutionOptions) => {
    const result = typescript.resolveModuleName(specifier, containingFile, resolutionOptions, resolutionHost);
    noteResolvedTarget(specifier, containingFile, result.resolvedModule?.resolvedFileName ?? null);
    return result.resolvedModule;
  };
  host.resolveModuleNames = (moduleNames, containingFile, _reused, _redirected, resolutionOptions) => moduleNames.map((name) => resolveOne(name, containingFile, resolutionOptions ?? options));
  host.resolveModuleNameLiterals = (literals, containingFile, _redirected, resolutionOptions) => literals.map((literal) => ({ resolvedModule: resolveOne(literal.text, containingFile, resolutionOptions ?? options) }));
  let program;
  try {
    program = typescript.createProgram({ rootNames: parsed.fileNames, options, host });
  } catch (error) {
    if (state.aborted === "unauthorized") {
      return basePayload("unauthorized", false, "rejected", state.unauthorized.at(-1)?.reason ?? policy.reasons.outsideBoundary, {
        compilerVersion: version,
        moduleResolution: modeName,
        unauthorized: state.unauthorized,
        timingsMs: { config: configMs, program: Date.now() - started - configMs, graph: 0 },
      });
    }
    if (state.aborted === "unsupported") {
      return basePayload("unsupported", false, "rejected", state.unsupported.at(-1)?.reason ?? error.message, {
        compilerVersion: version,
        moduleResolution: modeName,
        timingsMs: { config: configMs, program: Date.now() - started - configMs, graph: 0 },
      });
    }
    throw error;
  }
  const programMs = Date.now() - started - configMs;
  if (state.unauthorized.length > 0) {
    return basePayload("unauthorized", false, "rejected", state.unauthorized.at(-1).reason, {
      compilerVersion: version,
      moduleResolution: modeName,
      unauthorized: state.unauthorized,
      timingsMs: { config: configMs, program: programMs, graph: 0 },
    });
  }
  const sources = [];
  const sourceFiles = program.getSourceFiles();
  if (sourceFiles.length > request.limits.maxSources) {
    return basePayload("unsupported", false, "partial", "Compiler program exceeded the source limit. The graph is incomplete.", {
      compilerVersion: version,
      moduleResolution: modeName,
      timingsMs: { config: configMs, program: programMs, graph: 0 },
    });
  }
  for (const sourceFile of sourceFiles) {
    const logicalPath = path.resolve(sourceFile.fileName);
    const real = canonicalPath(logicalPath) ?? logicalPath;
    if (isInside(policy.typescriptLibRoot, real)) {
      sources.push({ logicalPath, canonicalPath: real, role: "library" });
      continue;
    }
    if (isVirtual(logicalPath)) {
      return basePayload("unsupported", false, "rejected", policy.reasons.virtualIdentity, {
        compilerVersion: version,
        moduleResolution: modeName,
        sources,
        timingsMs: { config: configMs, program: programMs, graph: 0 },
      });
    }
    sources.push({ logicalPath, canonicalPath: real, role: classify(logicalPath, policy) });
  }
  const edges = [];
  const failures = [];
  const seen = new Set();
  for (const sourceFile of sourceFiles) {
    if (isInside(policy.typescriptLibRoot, canonicalPath(sourceFile.fileName) ?? sourceFile.fileName)) {
      continue;
    }
    for (const spec of collectSpecifiers(sourceFile, typescript)) {
      const key = `${sourceFile.fileName}\0${spec.kind}\0${spec.specifier}`;
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      if (edges.length >= request.limits.maxEdges) {
        return basePayload("unsupported", false, "partial", "Compiler resolution edge limit was reached. The graph is incomplete.", {
          compilerVersion: version,
          moduleResolution: modeName,
          sources,
          edges,
          resolutionFailures: failures,
          timingsMs: { config: configMs, program: programMs, graph: Date.now() - started - configMs - programMs },
        });
      }
      let target = null;
      if (spec.reference && typescript.resolveTripleslashReference) {
        target = typescript.resolveTripleslashReference(spec.specifier, sourceFile.fileName);
      } else if (spec.typeReference && typescript.resolveTypeReferenceDirective) {
        const directive = typescript.resolveTypeReferenceDirective(spec.specifier, sourceFile.fileName, options, resolutionHost);
        target = resolvedFile(directive);
      } else {
        const resolved = typescript.resolveModuleName(spec.specifier, sourceFile.fileName, options, resolutionHost);
        target = resolved.resolvedModule?.resolvedFileName ?? null;
        if (!target) {
          failures.push({ from: path.resolve(sourceFile.fileName), specifier: spec.specifier, kind: spec.kind });
        }
      }
      if (target) {
        const decision = authorize(target, "exists", policy);
        if (decision.action === "deny") {
          remember(state.unauthorized, { ...decision.record, from: path.resolve(sourceFile.fileName), specifier: spec.specifier });
        }
        if (decision.action === "unsupported") {
          remember(state.unsupported, decision.record);
        }
      }
      edges.push({
        from: path.resolve(sourceFile.fileName),
        specifier: spec.specifier,
        to: target ? path.resolve(target) : null,
        canonicalTo: target ? canonicalPath(target) ?? path.resolve(target) : null,
        kind: spec.kind,
        resolved: Boolean(target),
      });
    }
  }
  const graphMs = Date.now() - started - configMs - programMs;
  if (state.unauthorized.length > 0) {
    return basePayload("unauthorized", false, "rejected", state.unauthorized.at(-1).reason, {
      compilerVersion: version,
      moduleResolution: modeName,
      sources,
      edges,
      resolutionFailures: failures,
      unauthorized: state.unauthorized,
      timingsMs: { config: configMs, program: programMs, graph: graphMs },
    });
  }
  if (state.unsupported.length > 0) {
    return basePayload("unsupported", false, "rejected", state.unsupported.at(-1).reason, {
      compilerVersion: version,
      moduleResolution: modeName,
      sources,
      edges,
      resolutionFailures: failures,
      timingsMs: { config: configMs, program: programMs, graph: graphMs },
    });
  }
  return basePayload("ok", true, "complete", "Compiler resolution evidence is complete.", {
    compilerVersion: version,
    moduleResolution: modeName,
    sources,
    edges,
    resolutionFailures: failures,
    unauthorized: [],
    timingsMs: { config: configMs, program: programMs, graph: graphMs },
  });
}

try {
  const request = JSON.parse(fs.readFileSync(0, "utf8"));
  emit(run(request));
} catch (error) {
  const status = state.aborted === "unauthorized" ? "unauthorized" : state.aborted === "unsupported" ? "unsupported" : "malformed";
  emit(basePayload(status, false, "rejected", error && error.stack ? error.stack : String(error), {
    unauthorized: state.unauthorized,
  }));
}
