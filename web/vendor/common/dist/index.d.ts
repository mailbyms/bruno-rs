declare const timeBasedDynamicVars: Set<string>;
declare const mockDataFunctions: {
    guid: () => string;
    timestamp: () => string;
    isoTimestamp: () => string;
    randomUUID: () => string;
    randomNanoId: () => string;
    randomAlphaNumeric: () => string;
    randomBoolean: () => boolean;
    randomInt: () => number;
    randomColor: () => string;
    randomHexColor: () => string;
    randomAbbreviation: () => string;
    randomIP: () => string;
    randomIPV4: () => string;
    randomIPV6: () => string;
    randomMACAddress: () => string;
    randomPassword: () => string;
    randomLocale: () => string;
    randomUserAgent: () => string;
    randomProtocol: () => "http" | "https";
    randomSemver: () => string;
    randomFirstName: () => string;
    randomLastName: () => string;
    randomFullName: () => string;
    randomNamePrefix: () => string;
    randomNameSuffix: () => string;
    randomJobArea: () => string;
    randomJobDescriptor: () => string;
    randomJobTitle: () => string;
    randomJobType: () => string;
    randomPhoneNumber: () => string;
    randomPhoneNumberExt: () => string;
    randomCity: () => string;
    randomStreetName: () => string;
    randomStreetAddress: () => string;
    randomCountry: () => string;
    randomCountryCode: () => string;
    randomLatitude: () => number;
    randomLongitude: () => number;
    randomAvatarImage: () => string;
    randomImageUrl: () => string;
    randomAbstractImage: () => string;
    randomAnimalsImage: () => string;
    randomBusinessImage: () => string;
    randomCatsImage: () => string;
    randomCityImage: () => string;
    randomFoodImage: () => string;
    randomNightlifeImage: () => string;
    randomFashionImage: () => string;
    randomPeopleImage: () => string;
    randomNatureImage: () => string;
    randomSportsImage: () => string;
    randomTransportImage: () => string;
    randomImageDataUri: () => string;
    randomBankAccount: () => string;
    randomBankAccountName: () => string;
    randomCreditCardMask: () => string;
    randomBankAccountBic: () => string;
    randomBankAccountIban: () => string;
    randomTransactionType: () => string;
    randomCurrencyCode: () => string;
    randomCurrencyName: () => string;
    randomCurrencySymbol: () => string;
    randomBitcoin: () => string;
    randomCompanyName: () => string;
    randomCompanySuffix: () => string;
    randomBs: () => string;
    randomBsAdjective: () => string;
    randomBsBuzz: () => string;
    randomBsNoun: () => string;
    randomCatchPhrase: () => string;
    randomCatchPhraseAdjective: () => string;
    randomCatchPhraseDescriptor: () => string;
    randomCatchPhraseNoun: () => string;
    randomDatabaseColumn: () => string;
    randomDatabaseType: () => string;
    randomDatabaseCollation: () => string;
    randomDatabaseEngine: () => string;
    randomDateFuture: () => string;
    randomDatePast: () => string;
    randomDateRecent: () => string;
    randomWeekday: () => string;
    randomMonth: () => string;
    randomDomainName: () => string;
    randomDomainSuffix: () => string;
    randomDomainWord: () => string;
    randomEmail: () => string;
    randomExampleEmail: () => string;
    randomUserName: () => string;
    randomUrl: () => string;
    randomFileName: () => string;
    randomFileType: () => string;
    randomFileExt: () => string;
    randomCommonFileName: () => string;
    randomCommonFileType: () => string;
    randomCommonFileExt: () => string;
    randomFilePath: () => string;
    randomDirectoryPath: () => string;
    randomMimeType: () => string;
    randomPrice: () => string;
    randomProduct: () => string;
    randomProductAdjective: () => string;
    randomProductMaterial: () => string;
    randomProductName: () => string;
    randomDepartment: () => string;
    randomNoun: () => string;
    randomVerb: () => string;
    randomIngverb: () => string;
    randomAdjective: () => string;
    randomWord: () => string;
    randomWords: () => string;
    randomPhrase: () => string;
    randomLoremWord: () => string;
    randomLoremWords: () => string;
    randomLoremSentence: () => string;
    randomLoremSentences: () => string;
    randomLoremParagraph: () => string;
    randomLoremParagraphs: () => string;
    randomLoremText: () => string;
    randomLoremSlug: () => string;
    randomLoremLines: () => string;
};

/**
 * The interpolation function expects a string with placeholders and an object with the values to replace the placeholders.
 * The keys passed can have dot notation too.
 *
 * Ex: interpolate('Hello, my name is ${user.name} and I am ${user.age} years old', {
 *  "user.name": "Bruno",
 *  "user": {
 *   "age": 6
 *  }
 * });
 * Output: Hello, my name is Bruno and I am 6 years old
 */
type InterpolateOptions = {
    escapeJSONStrings?: boolean;
    onUnresolved?: (name: string) => void;
};
declare const interpolate: (str: string, obj: Record<string, any>, options?: InterpolateOptions) => string;
declare const interpolateObject: (obj: unknown, variables: Record<string, any>) => unknown;

declare const percentageToZoomLevel: (percentage: number) => number;

/**
 * Shape of any node that can carry tags. A request node *is* its file, so its tags sit at the item
 * level and mirror into `draft.tags`. A folder node is a directory that may have no root file behind
 * it at all, so its tags stay on `root.meta.tags` - the in-memory image of `folder.bru`/`folder.yml` -
 * and a folder draft, being a clone of that root, carries them at `draft.meta.tags`.
 */
interface TaggedTreeNode {
    type?: string;
    name?: string;
    tags?: string[] | null;
    draft?: {
        tags?: string[] | null;
        meta?: {
            tags?: string[] | null;
        } | null;
    } | null;
    root?: {
        meta?: {
            tags?: string[] | null;
        } | null;
    } | null;
    [key: string]: unknown;
}
/** Trims, drops non-strings and empties, and de-duplicates while preserving order. */
declare const normalizeTags: (tags: unknown) => string[];
/** Tags a folder carries itself, draft-aware. */
declare const getFolderTags: (folder?: TaggedTreeNode | null) => string[];
/** Tags an item carries itself (no inheritance), draft-aware, for folders and requests alike. */
declare const getOwnTags: (item?: TaggedTreeNode | null) => string[];
interface InheritedTagSource {
    tag: string;
    folder: TaggedTreeNode;
}
/** Tags an item inherits from the folders above it, paired with their source folder */
declare const getInheritedTagSourcesFromTreePath: (treePath?: (TaggedTreeNode | null | undefined)[]) => InheritedTagSource[];
declare const getInheritedTagsFromTreePath: (treePath?: (TaggedTreeNode | null | undefined)[]) => string[];
/** Own tags plus those cascaded from parent folders; a request cannot opt out of folder tags. */
declare const getEffectiveTags: (ownTags: string[] | null | undefined, inheritedTags?: string[] | null | undefined) => string[];
/**
 * Included if the request has at least one included tag and no excluded tag. Callers pass effective
 * tags (see `getEffectiveTags`), not just the request's own.
 */
declare const isRequestTagsIncluded: (requestTags: string[], includeTags: string[], excludeTags: string[]) => boolean;

interface Collection {
    items?: any[];
    [key: string]: any;
}
/**
 * Backward compatibility: Convert string status to number in examples
 * Old collections exported before the fix had status as string
 * This function ensures status is always a number for schema validation
 */
declare const transformExampleStatusInCollection: (collection: Collection | Collection[]) => Collection;

interface SortableItem {
    name?: string;
    seq?: number;
}
declare const sortByNameThenSequence: <T extends SortableItem>(items: T[]) => T[];
declare const resolveCollectionVersion: (brunoConfig: {
    version?: string;
    collectionVersion?: string;
} | null | undefined, isOpenCollection: boolean) => string;

declare const toDisplayString: (value: unknown, fallback?: string, indentation?: number) => string;
/**
 * Converts CRLF and lone CR line endings to LF.
 * Non-string input (e.g. a YAML scalar parsed as a number) is returned untouched.
 */
declare const normalizeLineEndings: (text: string) => string;

declare const AUTH_PLACEMENTS: readonly ["request", "collection"];
declare const REQUEST_NAME_SOURCES: readonly ["summary", "operationId", "description", "path"];
type AuthPlacement = typeof AUTH_PLACEMENTS[number];
type RequestNameSource = typeof REQUEST_NAME_SOURCES[number];
declare const DEFAULT_AUTH_PLACEMENT: AuthPlacement;
declare const DEFAULT_REQUEST_NAME_SOURCE: RequestNameSource;
interface OpenApiSyncConfig {
    sourceUrl: string;
    groupBy?: 'tags' | 'path';
    authPlacement?: AuthPlacement;
    requestNameSource?: RequestNameSource;
    lastSyncDate?: string;
    specHash?: string;
    autoCheck: boolean;
    autoCheckInterval: number;
}
declare const normalizeOpenApiSyncConfigs: (entries: unknown) => OpenApiSyncConfig[];

interface TaggedItem {
    type?: string;
    tags?: string[];
    items?: TaggedItem[];
    [key: string]: unknown;
}
interface NamedEnvironment {
    name?: string;
    [key: string]: unknown;
}
interface DocsCollection {
    name?: string;
    items?: TaggedItem[];
    environments?: NamedEnvironment[];
    [key: string]: unknown;
}
interface DocsOpenCollection {
    info?: {
        name?: string;
        version?: string;
        [key: string]: unknown;
    };
    extensions?: {
        bruno?: Record<string, unknown>;
        [key: string]: unknown;
    };
    [key: string]: unknown;
}
interface ApiDocsMetadata {
    collectionVersion?: string | number | null;
    exportedAt?: string;
    exportedUsing?: string;
}
interface ApiDocsHtmlOptions {
    gitCollectionUrl?: string | null;
    rendererBaseUrl?: string;
}
interface GenerateApiDocsOptions extends ApiDocsMetadata {
    tags?: {
        include?: string[];
        exclude?: string[];
    };
    environments?: {
        include?: string[];
        exclude?: string[];
    };
    gitCollectionUrl?: string | null;
    rendererBaseUrl?: string;
}
interface ApiDocsDependencies {
    brunoToOpenCollection: (collection: DocsCollection) => DocsOpenCollection;
    dumpYaml: (value: DocsOpenCollection, options: object) => string;
    escapeString: (value: string, options: object) => string;
}
declare const filterRequestItemsByTags: <T extends TaggedItem>(items: T[], includeTags?: string[], excludeTags?: string[], inheritedTags?: string[]) => T[];
declare const getApiDocsFileName: (collectionName: string | null | undefined) => string;
declare const generateApiDocsHtml: (collection: DocsCollection, options: GenerateApiDocsOptions, deps: ApiDocsDependencies) => string;

declare const HTTP_SCRIPT_KEYS: string[];
declare const GRPC_SCRIPT_KEYS: string[];
declare const SCRIPT_TYPES: Readonly<{
    readonly PRE_REQUEST: "pre-request";
    readonly POST_RESPONSE: "post-response";
    readonly TEST: "test";
    readonly BEFORE_CALL_START: "before-call-start";
    readonly BEFORE_MESSAGE_SEND: "before-message-send";
    readonly AFTER_MESSAGE_RECEIVE: "after-message-receive";
    readonly AFTER_CALL_END: "after-call-end";
}>;

/**
 * Bruno → HAR generation.
 *
 * This module is the single canonical pipeline for turning a Bruno request
 * (interpolation, path-param substitution, auth resolution, header merging,
 * URL encoding, query handling, HAR assembly) into the HTTP Archive (HAR)
 * shape consumed by both Generate Code (HTTPSnippet) and — in a follow-up
 * PR — the runtime (via a HAR→axios adapter).
 *
 * The previous architecture spread these concerns across 5+ files in
 * `bruno-app` plus a parallel set in `bruno-electron`/`bruno-cli`, and the
 * runtime↔codegen drift produced every reported URL-encoding bug
 * (#5788, #6268, #7356, #7653, #7913, plus the #tag#tag doubling and the
 * `post_ids[]` / `key[a][b]` bracket-phantom-duplicate bugs).
 *
 * By centralising the work here:
 *   - There is exactly one place that decides what reaches the wire.
 *   - PR #5507's content-blind double-encoding contract is honored by
 *     calling `encodeUrl()` from `../utils/url` (no decode-then-encode).
 *   - The HAR `url` field is path-only; `queryString` is the sole source
 *     of truth for the rendered query string, which sidesteps HTTPSnippet's
 *     legacy `url.parse(..., true, true)` polyfill that strips trailing
 *     `[]` from keys (the bracket-phantom bug).
 *   - `{{var}}` tokens that callers don't want resolved are hashed before
 *     URL parsing/encoding via `patternHasher`, then restored at the end
 *     via the returned `unhash` function — so snippets remain
 *     copy-shareable with templates intact.
 */
type BrunoKV = {
    name: string;
    value: string;
    type?: string;
    enabled?: boolean;
};
type BrunoBody = {
    mode?: string;
    [key: string]: any;
};
type BrunoAuth = {
    mode?: string;
    [key: string]: any;
};
type BrunoRequest = {
    method?: string;
    url: string;
    params?: BrunoKV[];
    pathParams?: BrunoKV[];
    headers?: BrunoKV[];
    body?: BrunoBody;
    auth?: BrunoAuth;
    settings?: {
        encodeUrl?: boolean;
    };
};
/**
 * Stored OAuth2 credential record (read from `collection.oauth2Credentials`).
 * Passed in as input so `buildHar` stays pure (no collection traversal);
 * the caller pre-collects the records relevant to this request.
 */
type OAuth2CredentialRecord = {
    url?: string;
    collectionUid?: string;
    credentialsId?: string;
    credentials?: {
        access_token?: string;
    };
};
type BuildHarInput = {
    request: BrunoRequest;
    /**
     * Merged variable map: global / collection / folder / request / runtime /
     * oauth2-creds / prompt vars / process.env, in precedence order resolved
     * by the caller (the caller is the only place that knows the collection
     * tree). `buildHar` just substitutes from this map.
     */
    variables?: Record<string, unknown>;
    /**
     * When `false` (default for Generate Code's "show snippet without
     * resolving variables" mode), `{{var}}` tokens are hashed before URL
     * parsing/encoding via `patternHasher`, then exposed via the returned
     * `unhash` function so the caller can restore them in the final output.
     */
    shouldInterpolate?: boolean;
    /**
     * OAuth2 stored credentials. Used to look up the actual access token
     * when `auth.mode === 'oauth2'` and `tokenPlacement === 'header'`.
     */
    oauth2Credentials?: OAuth2CredentialRecord[];
    collectionUid?: string;
};
type HarRequest = {
    method: string;
    url: string;
    httpVersion: string;
    cookies: unknown[];
    headers: {
        name: string;
        value: string;
    }[];
    queryString: {
        name: string;
        value: string;
    }[];
    postData: any;
    headersSize: number;
    bodySize: number;
    binary: boolean;
};
type BuildHarOutput = {
    /** Canonical HAR object, ready for HTTPSnippet (and, post-Phase-C, an axios adapter). */
    har: HarRequest;
    /**
     * The user's URL after path-param substitution but BEFORE `encodeUrl`
     * was applied. Used by Generate Code's display-swap step when toggle
     * is off (snippet shows the user's typed encoding, not Bruno's).
     */
    rawUrl: string;
    /** The URL after `encodeUrl` was applied (matches the HAR's stored form). */
    encodedUrl: string;
    /**
     * Restores `{{var}}` tokens in a string. When `shouldInterpolate=false`,
     * Generate Code calls this on the final snippet so templates survive
     * verbatim through HTTPSnippet's URL parsing.
     */
    unhash: (input: string) => string;
    /**
     * The auth object as it was applied to the request. Exposed so Generate
     * Code can attach curl-specific flags (`--digest`, `--ntlm`) downstream.
     */
    effectiveAuth: BrunoAuth | null;
};
declare function buildHar(input: BuildHarInput): Promise<BuildHarOutput>;

type BrunoDefaultHeaderSource = 'bruno' | 'axios' | 'node';
type BrunoDefaultHeader = {
    name: string;
    /** Value shown in the Headers UI. */
    previewValue?: string;
    /** False for required defaults such as Host. */
    omittable: boolean;
    source: BrunoDefaultHeaderSource;
};
/** Headers Bruno, Axios, or Node add automatically on HTTP sends. */
declare const BRUNO_DEFAULT_HEADERS: BrunoDefaultHeader[];
declare const getBrunoDefaultHeaderNames: () => string[];
type RequestWithExplicitHeaderNames = {
    headers?: Record<string, unknown>;
    __explicitHeaderNames?: string[];
};
/** Headers the user or scripts set, not Axios defaults. Refresh before send. */
declare const refreshExplicitHeaderNames: <T extends RequestWithExplicitHeaderNames>(request: T) => T;
type ApplyOmitHeadersResult = {
    /** True when Connection should be stripped after agents are attached. */
    omitConnection: boolean;
};
type AxiosLikeHeaders = {
    set: (name: string, value: unknown) => void;
};
type OmitHeadersOptions = {
    omitHeaders?: string[] | null;
    headersToDelete?: string[] | null;
    explicitHeaderNames?: string[] | null;
};
/**
 * True when Connection must stay off the wire. Deleting the header is not enough
 * Node's agent writes it, so the agent has to be built with keepAlive false.
 */
declare const shouldOmitConnection: (options?: OmitHeadersOptions) => boolean;
/** Drop auto-added headers from an Axios request. Connection is left for the caller. */
declare const applyOmitHeaders: (headers: AxiosLikeHeaders, options?: OmitHeadersOptions) => ApplyOmitHeadersResult;
/** User-Agent Bruno sends by default. */
declare const getBrunoRuntimeUserAgent: (version: string) => string;

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

declare const index_BRUNO_VARIABLE_DATATYPES: typeof BRUNO_VARIABLE_DATATYPES;
type index_BrunoVariableDataType = BrunoVariableDataType;
declare const index_DEFAULT_MAX_REDIRECTS: typeof DEFAULT_MAX_REDIRECTS;
declare const index_DEFAULT_SCHEME: typeof DEFAULT_SCHEME;
type index_DotenvVariable = DotenvVariable;
declare const index_MAX_BODY_SIZE_DEFAULT: typeof MAX_BODY_SIZE_DEFAULT;
declare const index_PROMPT_VARIABLE_TEMPLATE_PATTERN: typeof PROMPT_VARIABLE_TEMPLATE_PATTERN;
declare const index_PROMPT_VARIABLE_TEXT_PATTERN: typeof PROMPT_VARIABLE_TEXT_PATTERN;
declare const index_TIMEOUT_INHERIT: typeof TIMEOUT_INHERIT;
declare const index_base64HmacSha256: typeof base64HmacSha256;
declare const index_base64Sha256: typeof base64Sha256;
declare const index_buildFormUrlEncodedPayload: typeof buildFormUrlEncodedPayload;
declare const index_buildQueryString: typeof buildQueryString;
declare const index_canonicalizeHeaders: typeof canonicalizeHeaders;
declare const index_coerceEnvName: typeof coerceEnvName;
declare const index_dedupeImportedSecrets: typeof dedupeImportedSecrets;
declare const index_encodeUrl: typeof encodeUrl;
declare const index_extractBoundaryFromContentType: typeof extractBoundaryFromContentType;
declare const index_extractMockRoutePath: typeof extractMockRoutePath;
declare const index_extractPromptVariables: typeof extractPromptVariables;
declare const index_extractPromptVariablesFromString: typeof extractPromptVariablesFromString;
declare const index_generateUniqueName: typeof generateUniqueName;
declare const index_getDataTypeFromValue: typeof getDataTypeFromValue;
declare const index_getDuplicateSecretNames: typeof getDuplicateSecretNames;
declare const index_getExplicitScheme: typeof getExplicitScheme;
declare const index_getFilenameWithFormat: typeof getFilenameWithFormat;
declare const index_getInheritableEnvironments: typeof getInheritableEnvironments;
declare const index_getInheritedEnvironments: typeof getInheritedEnvironments;
declare const index_getMediaType: typeof getMediaType;
declare const index_getMockResponseRouteKey: typeof getMockResponseRouteKey;
declare const index_hasExplicitScheme: typeof hasExplicitScheme;
declare const index_isBrunoVariableDataType: typeof isBrunoVariableDataType;
declare const index_isFormData: typeof isFormData;
declare const index_isSameOrigin: typeof isSameOrigin;
declare const index_isStrPresent: typeof isStrPresent;
declare const index_isValidNpmPackageName: typeof isValidNpmPackageName;
declare const index_jsonToDotenv: typeof jsonToDotenv;
declare const index_makeContentHash: typeof makeContentHash;
declare const index_makeEdgeGridNonce: typeof makeEdgeGridNonce;
declare const index_makeEdgeGridTimestamp: typeof makeEdgeGridTimestamp;
declare const index_nextCopyName: typeof nextCopyName;
declare const index_parseMaxRedirects: typeof parseMaxRedirects;
declare const index_parseQueryParams: typeof parseQueryParams;
declare const index_parseValueByDataType: typeof parseValueByDataType;
declare const index_patternHasher: typeof patternHasher;
declare const index_resolveEnvironmentInheritance: typeof resolveEnvironmentInheritance;
declare const index_resolveTimeoutSetting: typeof resolveTimeoutSetting;
declare const index_resolveVariableReference: typeof resolveVariableReference;
declare const index_safeDecodeURIComponent: typeof safeDecodeURIComponent;
declare const index_sanitizeName: typeof sanitizeName;
declare const index_stripCopySuffix: typeof stripCopySuffix;
declare const index_stripOrigin: typeof stripOrigin;
declare const index_toBool: typeof toBool;
declare const index_toDisplayString: typeof toDisplayString;
declare const index_toMaxRedirects: typeof toMaxRedirects;
declare const index_toNumber: typeof toNumber;
declare const index_toVariablesMap: typeof toVariablesMap;
declare const index_validateDataTypeValue: typeof validateDataTypeValue;
declare const index_validateName: typeof validateName;
declare const index_validateNameError: typeof validateNameError;
declare const index_validateVariableType: typeof validateVariableType;
declare const index_validatedEnvironmentExtendsFrom: typeof validatedEnvironmentExtendsFrom;
declare const index_validatedEnvironmentName: typeof validatedEnvironmentName;
declare const index_valueToString: typeof valueToString;
declare namespace index {
  export {
    index_BRUNO_VARIABLE_DATATYPES as BRUNO_VARIABLE_DATATYPES,
    index_BrunoVariableDataType as BrunoVariableDataType,
    index_DEFAULT_MAX_REDIRECTS as DEFAULT_MAX_REDIRECTS,
    index_DEFAULT_SCHEME as DEFAULT_SCHEME,
    index_DotenvVariable as DotenvVariable,
    index_MAX_BODY_SIZE_DEFAULT as MAX_BODY_SIZE_DEFAULT,
    index_PROMPT_VARIABLE_TEMPLATE_PATTERN as PROMPT_VARIABLE_TEMPLATE_PATTERN,
    index_PROMPT_VARIABLE_TEXT_PATTERN as PROMPT_VARIABLE_TEXT_PATTERN,
    index_TIMEOUT_INHERIT as TIMEOUT_INHERIT,
    index_base64HmacSha256 as base64HmacSha256,
    index_base64Sha256 as base64Sha256,
    index_buildFormUrlEncodedPayload as buildFormUrlEncodedPayload,
    index_buildQueryString as buildQueryString,
    index_canonicalizeHeaders as canonicalizeHeaders,
    index_coerceEnvName as coerceEnvName,
    index_dedupeImportedSecrets as dedupeImportedSecrets,
    index_encodeUrl as encodeUrl,
    index_extractBoundaryFromContentType as extractBoundaryFromContentType,
    index_extractMockRoutePath as extractMockRoutePath,
    index_extractPromptVariables as extractPromptVariables,
    index_extractPromptVariablesFromString as extractPromptVariablesFromString,
    index_generateUniqueName as generateUniqueName,
    index_getDataTypeFromValue as getDataTypeFromValue,
    index_getDuplicateSecretNames as getDuplicateSecretNames,
    index_getExplicitScheme as getExplicitScheme,
    index_getFilenameWithFormat as getFilenameWithFormat,
    index_getInheritableEnvironments as getInheritableEnvironments,
    index_getInheritedEnvironments as getInheritedEnvironments,
    index_getMediaType as getMediaType,
    index_getMockResponseRouteKey as getMockResponseRouteKey,
    index_hasExplicitScheme as hasExplicitScheme,
    index_isBrunoVariableDataType as isBrunoVariableDataType,
    index_isFormData as isFormData,
    index_isSameOrigin as isSameOrigin,
    index_isStrPresent as isStrPresent,
    index_isValidNpmPackageName as isValidNpmPackageName,
    index_jsonToDotenv as jsonToDotenv,
    index_makeContentHash as makeContentHash,
    index_makeEdgeGridNonce as makeEdgeGridNonce,
    index_makeEdgeGridTimestamp as makeEdgeGridTimestamp,
    index_nextCopyName as nextCopyName,
    index_parseMaxRedirects as parseMaxRedirects,
    index_parseQueryParams as parseQueryParams,
    index_parseValueByDataType as parseValueByDataType,
    index_patternHasher as patternHasher,
    index_resolveEnvironmentInheritance as resolveEnvironmentInheritance,
    index_resolveTimeoutSetting as resolveTimeoutSetting,
    index_resolveVariableReference as resolveVariableReference,
    index_safeDecodeURIComponent as safeDecodeURIComponent,
    index_sanitizeName as sanitizeName,
    index_stripCopySuffix as stripCopySuffix,
    index_stripOrigin as stripOrigin,
    index_toBool as toBool,
    index_toDisplayString as toDisplayString,
    index_toMaxRedirects as toMaxRedirects,
    index_toNumber as toNumber,
    index_toVariablesMap as toVariablesMap,
    index_validateDataTypeValue as validateDataTypeValue,
    index_validateName as validateName,
    index_validateNameError as validateNameError,
    index_validateVariableType as validateVariableType,
    index_validatedEnvironmentExtendsFrom as validatedEnvironmentExtendsFrom,
    index_validatedEnvironmentName as validatedEnvironmentName,
    index_valueToString as valueToString,
  };
}

declare const DEFAULT_HTTP_ITEM_SETTINGS: {
    encodeUrl: boolean;
    forwardAuthorizationHeader: boolean;
};

export { AUTH_PLACEMENTS, ApiDocsDependencies, ApiDocsHtmlOptions, ApiDocsMetadata, ApplyOmitHeadersResult, BRUNO_DEFAULT_HEADERS, BrunoAuth, BrunoBody, BrunoDefaultHeader, BrunoDefaultHeaderSource, BrunoKV, BrunoRequest, BuildHarInput, BuildHarOutput, DEFAULT_AUTH_PLACEMENT, DEFAULT_HTTP_ITEM_SETTINGS, DEFAULT_REQUEST_NAME_SOURCE, DocsCollection, DocsOpenCollection, GRPC_SCRIPT_KEYS, GenerateApiDocsOptions, HTTP_SCRIPT_KEYS, HarRequest, InheritedTagSource, NamedEnvironment, OAuth2CredentialRecord, OmitHeadersOptions, REQUEST_NAME_SOURCES, SCRIPT_TYPES, TaggedItem, TaggedTreeNode, applyOmitHeaders, buildHar, filterRequestItemsByTags, generateApiDocsHtml, getApiDocsFileName, getBrunoDefaultHeaderNames, getBrunoRuntimeUserAgent, getEffectiveTags, getFolderTags, getInheritedTagSourcesFromTreePath, getInheritedTagsFromTreePath, getOwnTags, interpolate, interpolateObject, isRequestTagsIncluded, mockDataFunctions, normalizeLineEndings, normalizeOpenApiSyncConfigs, normalizeTags, percentageToZoomLevel, refreshExplicitHeaderNames, resolveCollectionVersion, shouldOmitConnection, sortByNameThenSequence, timeBasedDynamicVars, transformExampleStatusInCollection, index as utils };
