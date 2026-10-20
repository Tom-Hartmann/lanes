// Clip thumbnails: cached per path and fetched at most two at a time, so
// scrolling a large library never floods the backend with ffmpeg work.

import { api } from "../../lib/ipc";

const MAX_CONCURRENT = 2;
const MAX_CACHED = 400;

const cache = new Map<string, Promise<string>>();
const queue: (() => void)[] = [];
let active = 0;

function limit<T>(fn: () => Promise<T>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const run = () => {
      active++;
      fn()
        .then(resolve, reject)
        .finally(() => {
          active--;
          queue.shift()?.();
        });
    };
    if (active < MAX_CONCURRENT) run();
    else queue.push(run);
  });
}

export function clipThumbnail(path: string): Promise<string> {
  let p = cache.get(path);
  if (!p) {
    p = limit(() => api.getClipThumbnail(path));
    // Failed thumbnails may succeed later (e.g. file still being written).
    p.catch(() => cache.delete(path));
    cache.set(path, p);
    if (cache.size > MAX_CACHED) cache.delete(cache.keys().next().value as string);
  }
  return p;
}

export const forgetThumbnail = (path: string) => cache.delete(path);
