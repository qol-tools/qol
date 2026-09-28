use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use objc2::AllocAnyThread;
use objc2_foundation::{NSDate, NSMetadataItem, NSNumber, NSString, NSURL};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Indexed {
    pub size: Option<u64>,
    pub added: Option<SystemTime>,
}

pub(super) fn facts(path: &Path) -> Indexed {
    let Some(path) = path.to_str() else {
        return Indexed::default();
    };
    objc2::rc::autoreleasepool(|_pool| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let Some(item) = NSMetadataItem::initWithURL(NSMetadataItem::alloc(), &url) else {
            return Indexed::default();
        };
        let value = |key: &str| item.valueForAttribute(&NSString::from_str(key));
        Indexed {
            size: value("kMDItemLogicalSize")
                .and_then(|value| value.downcast::<NSNumber>().ok())
                .map(|size| size.unsignedLongLongValue())
                .filter(|size| *size > 0),
            added: value("kMDItemDateAdded")
                .and_then(|value| value.downcast::<NSDate>().ok())
                .map(|date| date.timeIntervalSince1970())
                .filter(|seconds| *seconds > 0.0)
                .map(|seconds| UNIX_EPOCH + Duration::from_secs_f64(seconds)),
        }
    })
}
