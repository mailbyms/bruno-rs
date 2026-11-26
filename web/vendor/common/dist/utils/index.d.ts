/**
 * The scheme a schemeless URL is sent over. "localhost:8080/api" and
 * "{{host}}/api" (where the variable carries no scheme of its own) both go out
 * over plain HTTP, so this is what callers prepend when a URL has to be parsed
 * or displayed with an explicit scheme.
 */
declare const DEFAULT_SCHEME = "http://";
/**
 * Returns the explicit network scheme of `url` (without "://"), or null when it
 * has none.
 *
 * Per the WHATWG URL Standard, all network-fetch schemes (http, https, ftp,
 * ws, wss, file) require "://" — the authority component is mandatory.
 * This means "localhost:8080" is NOT a scheme: the colon separates host from
 * port, so callers should prepend `DEFAULT_SCHEME` to it.
 *
 * The scheme character set (ASCII alpha/digit/+/-/.) follows the WHATWG URL
 * scheme-state parser, which accepts the same characters as all major browsers.
 * @see https://url.spec.whatwg.org/#scheme-state
 *
 * @example
 * getExplicitScheme('https://example.com') // 'https'
 * getExplicitScheme('ftp://files.example') // 'ftp'
 * getExplicitScheme('localhost:8080')       // null — port colon, not scheme
 * getExplicitScheme('example.com/api')      // null — no scheme at all
 */
declare function getExplicitScheme(url: string): string | null;
/**
 * Returns true when `url` already carries an explicit network scheme.
 *
 * @example
 * hasExplicitScheme('https://example.com') // true
 * hasExplicitScheme('localhost:8080')       // false — port colon, not scheme
 */
declare function hasExplicitScheme(url: string): boolean;
interface QueryParam {
    name: string;
    value?: string;
}
interface BuildQueryStringOptions {
    encode?: boolean;
}
interface ExtractQueryParamsOptions {
    decode?: boolean;
    /**
     * When `true` (default), anything after `#` is discarded before splitting
     * on `&` — matches the URL-bar reducer's view where `#` is the fragment
     * delimiter. Pass `false` from callers that want `#` to round-trip as data
     * (e.g., `encodeUrl`, which encodes `#` to `%23` rather than treating it
     * as a structural fragment marker).
     */
    stripFragment?: boolean;
}
declare function buildQueryString(paramsArray: QueryParam[], { encode }?: BuildQueryStringOptions): string;
declare function parseQueryParams(query: string, { decode, stripFragment }?: ExtractQueryParamsOptions): QueryParam[];
declare const safeDecodeURIComponent: (s: string) => string;
declare const encodeUrl: (url: string) => string;
/**
 * Strip the origin (scheme + authority) from a URL, returning the path, query, and fragment.
 * Returns '/' if the URL has no path component.
 *
 * @example
 * stripOrigin('https://example.com/api/users?name=foo') // '/api/users?name=foo'
 * stripOrigin('http://localhost:3000')                   // '/'
 * stripOrigin('ftp://files.example.com/pub')             // '/pub'
 */
declare const stripOrigin: (url: string) => string;
/**
 * Normalize a Bruno request URL to a mock-server route path.
 * Strips scheme/host, leading `{{var}}`, query strings; turns remaining `{{var}}` into `:var`
 * so it matches Express-style route params. Callers that persist or display the user-typed URL
 * pass `preserveTemplateVars: true` to keep `{{var}}` intact.
 */
declare const extractMockRoutePath: (rawUrl: unknown, { preserveTemplateVars }?: {
    preserveTemplateVars?: boolean | undefined;
}) => string;
interface MockResponseRouteKeyInput {
    request?: {
        method?: string;
        url?: string;
    } | null;
    response?: {
        status?: number | string;
    } | null;
}
declare const getMockResponseRouteKey: (response?: MockResponseRouteKeyInput | null) => string;
declare const isSameOrigin: (url1: string, url2: string) => boolean;

/**
 * Builds a URL-encoded payload from various data formats
 *
 * This function handles multiple input formats:
 * - Array of objects with 'name' and 'value' properties (preserves order)
 *
 * @param data The request body data
 * @returns URL-encoded string suitable for application/x-www-form-urlencoded content type
 *
 * @example
 * // Array format (preserves order)
 * buildFormUrlEncodedPayload([{name: 'a', value: '1'}, {name: 'b', value: '2'}])
 * // Returns: 'a=1&b=2'
 */
declare const buildFormUrlEncodedPayload: (params: Array<{
    name: string;
    value: string | number | boolean | undefined;
}>) => string;
/**
 * Determines if the given object is a FormData instance.
 * Supports native FormData (Node 18+, browser) and the 'form-data' npm package.
 * @param obj - Object to check.
 * @returns True if obj is a FormData instance, false otherwise.
 */
declare const isFormData: (obj: unknown) => boolean;
/**
 * Extracts the media type from a Content-Type header value, without parameters and lowercased.
 * @param contentType - The Content-Type header value (e.g., "Application/JSON; charset=UTF-8")
 * @returns The media type (e.g., "application/json"), or '' if contentType is not a string
 */
declare const getMediaType: (contentType: unknown) => string;
/**
 * Extracts boundary parameter from a Content-Type header value.
 * @param contentType - The Content-Type header value (e.g., "multipart/mixed; boundary=my-boundary")
 * @returns The boundary value if found, or null if not present
 */
declare const extractBoundaryFromContentType: (contentType: unknown) => string | null;

/**
 * Was implemented specifically for request.url where the url might have variables
 * that might need to be sanitised before being passed to a URL validator that doesn't
 * allow special characters that bruno uses as variables (`{{var_name}}`)
 *
 * The function replaces the input string with a unique hash that can be restored
 * later by the helper returned by this function
 */
declare function patternHasher(input: string, pattern?: string | RegExp): {
    hashed: string;
    restore(current: string): string;
};

/**
 * Valid examples: "?Name", "?Prompt Var", "?x"
 * Invalid examples: "? Name", "?Name ", "?{{Name}}", "?{Name}"
 */
declare const PROMPT_VARIABLE_TEXT_PATTERN: RegExp;
/**
 * Valid matches: "{{?Name}}", "{{?Prompt Var}}", "{{?x}}"
 * Invalid: "{{? Name}}", "{{?Name }}", "{{?{Name}}}"
 */
declare const PROMPT_VARIABLE_TEMPLATE_PATTERN: RegExp;
/**
 * Extract prompt variables matching {{?<Prompt Text>}} from a string.
 * @param {string} str - The input string.
 * @returns {string[]} - An array of extracted prompt variables.
 */
declare const extractPromptVariablesFromString: (str: string) => string[];
/**
 * Extract prompt variables from an object.
 * @param {*} obj - The input object.
 * @returns {string[]} - An array of extracted prompt variables.
 */
declare function extractPromptVariables(obj: any): string[];

interface DotenvVariable {
    name: string;
    value?: string;
}
/**
 * Serializes an array of environment variables to .env file format.
 *
 * This is the inverse of dotenvToJson - it converts a variables array
 * back to .env file content that can be parsed by the dotenv package.
 *
 * Quoting strategy based on dotenv parser behavior:
 * - Unquoted: preserves \, ", ' literally. Only # is problematic (starts comment).
 * - Single-quoted: everything is literal. Cannot contain single quotes.
 * - Double-quoted: only \n and \r are expanded. Everything else is literal.
 *
 * Therefore:
 * - Values with actual newlines/carriage returns → double-quote + escape \n/\r
 * - Values with # but no ' → single-quote (literal)
 * - Values with # and ' → backtick-quote or double-quote fallback
 * - Values with leading/trailing whitespace → quote to prevent trimming
 * - Everything else → unquoted (preserves \, ", ' as-is)
 */
declare const jsonToDotenv: (variables: DotenvVariable[]) => string;

type BrunoVariableDataType = 'string' | 'number' | 'boolean' | 'object';
declare const BRUNO_VARIABLE_DATATYPES: readonly BrunoVariableDataType[];
declare const isBrunoVariableDataType: (t: unknown) => t is BrunoVariableDataType;
declare const resolveVariableReference: (value: any, resolvableVariables: Record<string, any>) => any;
declare const parseValueByDataType: (value: any, dataType?: BrunoVariableDataType, resolvableVariables?: Record<string, any>) => any;
declare const getDataTypeFromValue: (value: unknown) => BrunoVariableDataType;
declare const valueToString: (value: unknown, indent?: number) => string;
declare const validateDataTypeValue: (value: any, dataType?: BrunoVariableDataType) => string | null;
declare const validateVariableType: (value: any, dataType?: BrunoVariableDataType, resolvableVariables?: Record<string, any>) => string | null;

declare const toDisplayString: (value: unknown, fallback?: string, indentation?: number) => string;

/**
 * Shared name/filename utilities.
 *
 * Single source of truth for sanitizing display names into filesystem-safe
 * names and validating names. Consumed by the Electron main process and the
 * renderer (and re-exported from each package's local helper so existing import
 * paths keep working). `generateUniqueName` resolves collisions against a
 * caller-supplied check, so the Electron main process and the CLI share it;
 * filesystem-probing suffix resolution (`nextSuffixedName`) stays in the
 * Electron filesystem utils.
 */
/**
 * Make a name safe to use as a filesystem name.
 *
 * Behavior is intentionally unchanged from the previous duplicated copies:
 * interior spaces are preserved; only illegal characters are replaced with `-`,
 * and leading spaces/hyphens and trailing dots/spaces are trimmed.
 */
declare const sanitizeName: (name: string) => string;
/**
 * Returns true if `name` is a valid file/dir name.
 *
 * Reconciled version: guards against falsy input (the renderer copy guarded
 * this; the Electron copy did not and would throw on `undefined`).
 */
declare const validateName: (name: string) => boolean;
/**
 * Human-readable validation error for `name`, or '' if valid.
 */
declare const validateNameError: (name: string) => string;
/**
 * Returns `baseName`, or `baseName copy`, `baseName copy 2`, ... — the first that `checkExists`
 * reports as free.
 */
declare const generateUniqueName: (baseName: string, checkExists: (name: string) => boolean) => string;
/**
 * Strips every trailing ` copy` / ` copy N` token, so a clone of a clone keeps the original base
 * name instead of stacking another token. Falls back to `name` when stripping leaves nothing.
 */
declare const stripCopySuffix: (name: string) => string;
/**
 * Returns `<base> copy`, then `<base> copy 2`, ... — the first free among `existingNames`, where
 * `base` is `sourceName` with any trailing copy token removed.
 */
declare const nextCopyName: (sourceName: string, existingNames: string[]) => string;
declare const getFilenameWithFormat: (item: {
    name: string;
    filename?: string | null;
}, format: string) => string;

interface InheritedFrom {
    name: string;
    uid: string;
}
interface InheritableVariable {
    name: string;
    enabled?: boolean;
    secret?: boolean;
    inheritedFrom?: InheritedFrom;
}
interface ExtendableEnvironment {
    uid: string;
    name: string;
    variables: InheritableVariable[];
    extends?: string | null;
}
interface UnresolvedInheritance {
    missingInheritedEnvironmentName?: string | null;
    cyclicInheritancePath?: string[] | null;
}
type ResolvedEnvironment<E extends ExtendableEnvironment, Merge extends boolean = false> = Merge extends true ? E & UnresolvedInheritance : E & {
    inheritedVariables: E['variables'];
} & UnresolvedInheritance;
declare const validatedEnvironmentName: (reference: unknown) => string | undefined;
declare const validatedEnvironmentExtendsFrom: (environmentExtendsReference: unknown) => string | string[] | undefined;
/**
 * An environment's `extends` chain, root ancestor first, so later entries override earlier ones.
 * The walk stops at an unresolvable reference or at a name already seen, so a broken or cyclic
 * chain yields the ancestors found so far, alongside the reference that ended the walk — nothing
 * keeps an `extends` name pointing at an environment that still exists, or keeps a chain from
 * closing a loop.
 */
declare const getInheritedEnvironments: <E extends ExtendableEnvironment>({ environments, environment }: {
    environments: E[];
    environment: E;
}) => {
    inheritedEnvironments: E[];
    missingInheritedEnvironmentName: string | null;
    cyclicInheritancePath: string[] | null;
};
/**
 * The environments a target may inherit from: everything but itself and its descendants,
 * since picking one of those closes a cycle.
 */
declare const getInheritableEnvironments: <E extends ExtendableEnvironment>({ environments, targetEnvironment }: {
    environments: E[];
    targetEnvironment: E | undefined;
}) => E[];
declare const resolveEnvironmentInheritance: <E extends ExtendableEnvironment, Merge extends boolean = false>({ environments, targetEnvironment, merge }: {
    environments: E[];
    targetEnvironment: E | undefined;
    merge?: Merge | undefined;
}) => ResolvedEnvironment<E, Merge> | undefined;

interface EnvironmentVariable {
    name?: string;
    value?: unknown;
    enabled?: boolean;
    secret?: boolean;
}
/**
 * Flattens environment variables into a name → value map. Later entries override earlier ones, so
 * callers order the list by precedence (inherited variables ahead of the environment's own), and a
 * secret wins over a plain variable of the same name whichever order the two appear in.
 */
declare const toVariablesMap: (environmentVariables?: EnvironmentVariable[]) => Record<string, unknown>;
/**
 * Secret values are persisted in a name-keyed side store and re-attached on read by name, so two
 * secrets sharing a name collapse to one and the second loses its value. Names must therefore be
 * unique among secrets. Returns the set of names carried by more than one secret variable. Only
 * secret rows are counted, so callers need not filter by tab.
 */
declare const getDuplicateSecretNames: (variables?: EnvironmentVariable[] | null) => Set<string>;
/**
 * Strips the duplicate secrets out of an imported environment, so an import never lands the user
 * with a collision they did not author. Keeps whichever twin holds a value, since a Postman export
 * ships secret values inline.
 */
declare const dedupeImportedSecrets: <T extends EnvironmentVariable>(variables: T[]) => T[];

/**
 * Reads an environment name from untrusted input (an import file). Returns the name as text, or
 * `null` when there is no usable name: a blank string, or anything that is neither a string nor a
 * finite number.
 */
declare const coerceEnvName: (name: unknown) => string | null;

declare const toBool: (value: any, defaultValue?: boolean) => boolean;
declare const toNumber: (value: any, defaultValue?: number) => number;

declare const isValidNpmPackageName: (name: unknown) => name is string;

/**
 * Shared EG1-HMAC-SHA256 primitives, used by both the runtime signer
 * (packages/bruno-requests/src/auth/edgegrid-helper.js) and the Generate Code signer
 * (packages/bruno-common/src/generate-code/har/edgegrid.ts) so the signing logic lives in one
 * place. Uses the native Web Crypto API (`globalThis.crypto`) to stay browser-safe and
 * dependency-free; Web Crypto is async, so the hashing helpers return Promises. Kept byte-for-byte
 * compatible with Akamai's reference implementation (github.com/akamai/AkamaiOPEN-edgegrid-node).
 */
declare const MAX_BODY_SIZE_DEFAULT = 131072;
declare const isStrPresent: (str?: string | null) => str is string;
declare const base64HmacSha256: (data: string, key: string) => Promise<string>;
declare const base64Sha256: (data: string) => Promise<string>;
declare const makeEdgeGridTimestamp: () => string;
declare const makeEdgeGridNonce: () => string;
declare const canonicalizeHeaders: (headersToSign?: string | null, requestHeaders?: Record<string, string>) => string;
declare const makeContentHash: (method: string, bodyText: string | undefined, maxBodySize: number) => Promise<string>;

declare const TIMEOUT_INHERIT: "inherit";
declare const resolveTimeoutSetting: (value: unknown) => number | typeof TIMEOUT_INHERIT;
declare const DEFAULT_MAX_REDIRECTS = 5;
declare const parseMaxRedirects: (value: unknown) => number | undefined;
declare const toMaxRedirects: (value: unknown) => number;

export { BRUNO_VARIABLE_DATATYPES, BrunoVariableDataType, DEFAULT_MAX_REDIRECTS, DEFAULT_SCHEME, DotenvVariable, MAX_BODY_SIZE_DEFAULT, PROMPT_VARIABLE_TEMPLATE_PATTERN, PROMPT_VARIABLE_TEXT_PATTERN, TIMEOUT_INHERIT, base64HmacSha256, base64Sha256, buildFormUrlEncodedPayload, buildQueryString, canonicalizeHeaders, coerceEnvName, dedupeImportedSecrets, encodeUrl, extractBoundaryFromContentType, extractMockRoutePath, extractPromptVariables, extractPromptVariablesFromString, generateUniqueName, getDataTypeFromValue, getDuplicateSecretNames, getExplicitScheme, getFilenameWithFormat, getInheritableEnvironments, getInheritedEnvironments, getMediaType, getMockResponseRouteKey, hasExplicitScheme, isBrunoVariableDataType, isFormData, isSameOrigin, isStrPresent, isValidNpmPackageName, jsonToDotenv, makeContentHash, makeEdgeGridNonce, makeEdgeGridTimestamp, nextCopyName, parseMaxRedirects, parseQueryParams, parseValueByDataType, patternHasher, resolveEnvironmentInheritance, resolveTimeoutSetting, resolveVariableReference, safeDecodeURIComponent, sanitizeName, stripCopySuffix, stripOrigin, toBool, toDisplayString, toMaxRedirects, toNumber, toVariablesMap, validateDataTypeValue, validateName, validateNameError, validateVariableType, validatedEnvironmentExtendsFrom, validatedEnvironmentName, valueToString };
