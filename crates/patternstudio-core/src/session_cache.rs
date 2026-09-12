//! LRU cache of VTracer [`Session`]s for interactive re-tracing.
//!
//! Segmentation (clustering) is by far the most expensive pipeline stage. A
//! [`Session`] caches it and re-renders only the cheap downstream stages when a
//! non-clustering parameter changes, re-segmenting automatically only when the
//! clustering parameters actually change. Caching one [`Session`] per source
//! image is what makes live slider tuning feel instant: the first render pays
//! the clustering cost, and every subsequent render that only touches curve /
//! fit / optimize / palette parameters reuses the cached segmentation.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

use vtracer::{ColorImage, Session};

/// A bounded LRU cache mapping a source key to a reusable [`Session`].
///
/// The key is typically the source image path combined with its size and mtime
/// (see [`key_for_path`]), so a replaced file invalidates its entry. Creating a
/// session decodes the image; the expensive segmentation runs lazily on the
/// first render and is then cached inside the [`Session`].
pub struct SessionCache {
    map: HashMap<String, Arc<Mutex<Session>>>,
    order: VecDeque<String>,
    cap: usize,
}

impl SessionCache {
    /// Create a cache holding at most `cap` sessions (evicts least-recently
    /// used entries beyond that).
    pub fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap: cap.max(1),
        }
    }

    fn touch(&mut self, key: &str) {
        self.order.retain(|k| k != key);
        self.order.push_back(key.to_string());
    }

    /// Return the cached session for `key`, creating (and caching) it via
    /// `create` on a miss. `create` decodes the source image into a
    /// [`ColorImage`]; it is only invoked when the cache has no entry for
    /// `key` yet.
    pub fn get_or_create<F, E>(&mut self, key: &str, create: F) -> Result<Arc<Mutex<Session>>, E>
    where
        F: FnOnce() -> Result<ColorImage, E>,
    {
        if self.map.contains_key(key) {
            self.touch(key);
            return Ok(self.map.get(key).expect("checked above").clone());
        }
        let img = create()?;
        let session = Arc::new(Mutex::new(Session::new(img)));
        while self.map.len() >= self.cap {
            match self.order.pop_front() {
                Some(old) => {
                    self.map.remove(&old);
                }
                None => break,
            }
        }
        self.map.insert(key.to_string(), session.clone());
        self.order.push_back(key.to_string());
        Ok(session)
    }

    /// Number of sessions currently cached.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// A cache key for a source file: `path:size:mtime`, so a file that is replaced
/// (different size or modification time) gets a fresh session instead of a
/// stale one. Falls back to the bare path if metadata can't be read.
pub fn key_for_path(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(m) => {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("{}:{}:{}", path.display(), m.len(), mtime)
        }
        Err(_) => path.display().to_string(),
    }
}

/// A cache key for a source file *and* its segmentation parameters.
///
/// Unlike [`key_for_path`] (per-file), this also folds in the vtracer
/// [`SegmentKey`](vtracer::config::SegmentKey) — the clustering parameters that
/// decide whether a re-segmentation is needed. Keying sessions this way lets
/// each preset / clustering setting keep its own cached segmentation instead of
/// evicting the previous one, so pre-warming several presets doesn't churn the
/// cache (and toggling between two clustering settings stays fast).
pub fn key_for_source(path: &Path, seg: &vtracer::config::SegmentKey) -> String {
    format!("{}:{:?}", key_for_path(path), seg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vtracer::{Config, Preset};

    /// A synthetic image with solid color blocks plus a smooth gradient region,
    /// so clustering has real structure to find (and is therefore the dominant
    /// pipeline cost, as it is for real photos).
    fn synthetic_image(w: usize, h: usize) -> ColorImage {
        let mut img = ColorImage::new_w_h(w, h);
        for y in 0..h {
            for x in 0..w {
                let (r, g, b) = if x < w / 2 {
                    (200, 60, 60)
                } else if y < h / 2 {
                    (60, 160, 220)
                } else {
                    ((x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, 120)
                };
                img.set_pixel(x, y, &visioncortex::Color::new(r, g, b));
            }
        }
        img
    }

    #[test]
    fn session_reuses_segmentation_for_cheap_params() {
        let img = synthetic_image(600, 600);

        // One-shot full pipeline — what the old code did on *every* request.
        let cfg = Config::from_preset(Preset::Poster);
        let pipeline = cfg.build().unwrap();
        let t = std::time::Instant::now();
        let _ = pipeline.run(&img).unwrap();
        let one_shot = t.elapsed();

        // Session: the first render pays the segmentation cost...
        let mut session = Session::new(img.clone());
        let t = std::time::Instant::now();
        let _ = session.render(&cfg).unwrap();
        let first = t.elapsed();

        // ...then a cheap (non-clustering) param change reuses the cache.
        let mut cfg_cheap = cfg.clone();
        cfg_cheap.simplify = Some(2.0);
        cfg_cheap.corner_threshold = 90;
        let t = std::time::Instant::now();
        let _ = session.render(&cfg_cheap).unwrap();
        let cheap = t.elapsed();

        // A clustering param change re-segments (expected to be ~one_shot).
        let mut cfg_reseg = cfg.clone();
        cfg_reseg.filter_speckle = 8;
        let t = std::time::Instant::now();
        let _ = session.render(&cfg_reseg).unwrap();
        let reseg = t.elapsed();

        eprintln!(
            "one-shot (full pipeline): {:?}\nfirst render (segment):   {:?}\ncheap re-render:          {:?}\nre-segment:               {:?}",
            one_shot, first, cheap, reseg
        );

        // The cheap re-render skips segmentation, so it must beat a full run.
        assert!(
            cheap.as_nanos() < one_shot.as_nanos(),
            "cheap re-render ({cheap:?}) should beat one-shot ({one_shot:?})"
        );
        // Re-segmenting should cost at least as much as the cheap re-render.
        assert!(
            reseg.as_nanos() >= cheap.as_nanos(),
            "re-segment ({reseg:?}) should cost >= cheap re-render ({cheap:?})"
        );
    }

    #[test]
    fn cache_evicts_least_recently_used() {
        let mut cache = SessionCache::new(2);
        let make = || Ok::<ColorImage, String>(synthetic_image(8, 8));
        cache.get_or_create("a", make).unwrap();
        cache.get_or_create("b", make).unwrap();
        // Touch "a" so "b" becomes the least-recently-used.
        cache.get_or_create("a", make).unwrap();
        // Adding "c" should evict "b".
        cache.get_or_create("c", make).unwrap();
        assert_eq!(cache.len(), 2);
        // "a" and "c" survive; "b" is gone (a fresh create is required).
        assert!(cache.get_or_create("a", make).is_ok());
        assert!(cache.get_or_create("c", make).is_ok());
        // "b" was evicted, so creating it again must invoke the loader.
        let calls = std::cell::Cell::new(0);
        cache.get_or_create("b", || -> Result<ColorImage, String> {
            calls.set(calls.get() + 1);
            Ok(synthetic_image(8, 8))
        }).unwrap();
        assert_eq!(calls.get(), 1, "evicted key must be recreated");
    }
}
