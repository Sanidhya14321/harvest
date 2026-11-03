/** Transitional callback transport; ZIP/XML/rendering are owned by Rust. */
import { docxFinish, docxPrepare, docxPrepareFile } from "@harvest/pi-natives";

/** A mammoth-compatible diagnostic emitted while converting a document. */
export interface DocxMessage {
	readonly type: "warning" | "error";
	readonly message: string;
}

/** The HTML and diagnostics produced by a DOCX conversion. */
export interface DocxResult {
	readonly value: string;
	readonly messages: DocxMessage[];
}

/** An in-memory or filesystem DOCX input. */
export type DocxInput =
	| { readonly buffer: Uint8Array; readonly path?: never }
	| { readonly path: string; readonly buffer?: never };

/** An image exposed to a custom image converter. */
export interface DocxImage {
	readonly contentType: string;
	readonly altText: string;
	/** Read the image payload using mammoth's used encoding surface. */
	read(encoding: "base64"): Promise<string>;
}

/** HTML attributes returned by a custom image converter. */
export type ImageAttributes = Readonly<Record<string, string>>;

/** A callback that maps an embedded DOCX image to HTML attributes. */
export type ImageAttributeConverter = (image: DocxImage) => ImageAttributes | Promise<ImageAttributes>;

/** An image converter created by `images.imgElement`. */
export interface ImageConverter {
	readonly convert: ImageAttributeConverter;
}

/** Options supported by the behavior-compatible DOCX converter. */
export interface ConvertToHtmlOptions {
	readonly convertImage?: ImageConverter;
	readonly styleMap?: string | readonly string[];
	readonly includeDefaultStyleMap?: boolean;
	readonly signal?: AbortSignal;
}

/** Mammoth-shaped helpers for configuring embedded image conversion. */
export const images = {
	/** Wrap an image-to-attributes callback for `convertToHtml`. */
	imgElement(convert: ImageAttributeConverter): ImageConverter {
		return { convert };
	},
};

/** Convert using the shared native engine, preserving the legacy image callback. */
export async function convertToHtml(input: DocxInput, options: ConvertToHtmlOptions = {}): Promise<DocxResult> {
	options.signal?.throwIfAborted();
	const settings = JSON.stringify({
		styleMap: typeof options.styleMap === "string" ? options.styleMap.split(/\r?\n/) : (options.styleMap ?? []),
		includeDefaultStyleMap: options.includeDefaultStyleMap !== false,
	});
	const prepared = input.buffer
		? await docxPrepare(Buffer.from(input.buffer), settings, options.signal)
		: await docxPrepareFile(input.path, settings, options.signal);
	let attributes: ImageAttributes[] | null = null;
	if (options.convertImage) {
		const data = JSON.parse(prepared) as { images: Array<{ contentType: string; altText: string; base64: string }> };
		attributes = [];
		for (const image of data.images) {
			options.signal?.throwIfAborted();
			const result = await options.convertImage.convert({
				contentType: image.contentType,
				altText: image.altText,
				async read(encoding) {
					if (encoding !== "base64") throw new Error(`Unsupported image encoding: ${encoding}`);
					return image.base64;
				},
			});
			attributes.push(Object.fromEntries(Object.entries(result).filter(([, value]) => typeof value === "string")));
		}
	}
	options.signal?.throwIfAborted();
	return JSON.parse(await docxFinish(prepared, JSON.stringify(attributes), options.signal)) as DocxResult;
}

/** Mammoth-shaped default export for transitional consumers. */
export default { convertToHtml, images };
