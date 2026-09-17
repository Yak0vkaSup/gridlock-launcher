import { GetObjectCommand, S3Client } from "@aws-sdk/client-s3";
import { getSignedUrl } from "@aws-sdk/s3-request-presigner";

// Cloudflare R2 through the S3 API. Game builds live here, behind presigned URLs.
const bucket = () => process.env.R2_BUCKET ?? "gridlock";

let client: S3Client | null = null;
export function r2(): S3Client {
  if (!client) {
    client = new S3Client({
      region: "auto",
      endpoint: `https://${process.env.R2_ACCOUNT_ID}.r2.cloudflarestorage.com`,
      credentials: {
        accessKeyId: process.env.R2_ACCESS_KEY_ID ?? "",
        secretAccessKey: process.env.R2_SECRET_ACCESS_KEY ?? "",
      },
    });
  }
  return client;
}

export async function getJson<T>(key: string): Promise<T | null> {
  try {
    const res = await r2().send(new GetObjectCommand({ Bucket: bucket(), Key: key }));
    const text = await res.Body!.transformToString();
    return JSON.parse(text) as T;
  } catch (e) {
    if ((e as { name?: string })?.name === "NoSuchKey") return null;
    throw e;
  }
}

export function presign(key: string, seconds = 3600): Promise<string> {
  return getSignedUrl(r2(), new GetObjectCommand({ Bucket: bucket(), Key: key }), { expiresIn: seconds });
}

export type ManifestFile = { path: string; sha256: string; size: number; exec?: boolean; stored?: number };
export type Manifest = {
  version: string;
  platform: "windows" | "linux";
  commit?: string;
  created?: string;
  exec: string;
  total: number;
  files: ManifestFile[];
  /** absent = raw objects; "glb1" = block blobs (publish_build.py --format glb1) */
  format?: "glb1";
  block?: number;
  stored?: number;
};
