//! macOS 강화 항목: 신뢰 앱 목록이 빈 배열인 키체인 항목.
//! `SecAccessCreate`에 null이 아닌 빈 배열을 주면 생성한 앱을 포함해 어떤 앱도 확인 창 없이 읽지 못한다.
//! null이면 생성한 앱이 신뢰 앱이 되어 확인 창이 없다. 폐기 예정 API(`SecKeychainItem*`, `SecAccess*`)는 이 파일에만 둔다.
//! 설계: docs/design/router-key-security.md

#![allow(unsafe_code, deprecated)]

use std::ffi::c_void;
use std::ptr;

use core_foundation_sys::array::{
    CFArrayCreate, CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef, kCFTypeArrayCallBacks,
};
use core_foundation_sys::base::{CFRelease, CFTypeRef};
use core_foundation_sys::string::{CFStringCreateWithCString, CFStringRef, kCFStringEncodingUTF8};
use security_framework_sys::base::{
    SecAccessRef, SecKeychainAttribute, SecKeychainAttributeList, SecKeychainItemRef,
};
use security_framework_sys::keychain::{
    SecKeychainFindGenericPassword, SecKeychainGetUserInteractionAllowed,
    SecKeychainSetUserInteractionAllowed,
};
use security_framework_sys::keychain_item::{SecKeychainItemDelete, SecKeychainItemFreeContent};

use super::storage::{HardenedKeychain, ItemError};

type OsStatus = i32;

const ERR_SEC_USER_CANCELED: OsStatus = -128;
const ERR_SEC_AUTH_FAILED: OsStatus = -25293;
const ERR_SEC_INTERACTION_NOT_ALLOWED: OsStatus = -25308;
const ERR_SEC_ITEM_NOT_FOUND: OsStatus = -25300;
const ERR_SEC_DUPLICATE_ITEM: OsStatus = -25299;
/// 'genp'
const GENERIC_PASSWORD_CLASS: u32 = 0x6765_6E70;
/// 'svce'
const SERVICE_ATTR: u32 = 0x7376_6365;
/// 'acct'
const ACCOUNT_ATTR: u32 = 0x6163_6374;

type SecAclRef = *mut c_void;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecAccessCreate(
        descriptor: CFStringRef,
        trusted_list: CFArrayRef,
        access: *mut SecAccessRef,
    ) -> OsStatus;
    fn SecKeychainItemCreateFromContent(
        item_class: u32,
        attr_list: *const SecKeychainAttributeList,
        length: u32,
        data: *const c_void,
        keychain: *mut c_void,
        initial_access: SecAccessRef,
        item_ref: *mut SecKeychainItemRef,
    ) -> OsStatus;
    fn SecKeychainItemCopyAccess(item: SecKeychainItemRef, access: *mut SecAccessRef) -> OsStatus;
    fn SecAccessCopyMatchingACLList(access: SecAccessRef, tag: CFTypeRef) -> CFArrayRef;
    fn SecACLCopyContents(
        acl: SecAclRef,
        application_list: *mut CFArrayRef,
        description: *mut CFStringRef,
        prompt_selector: *mut u16,
    ) -> OsStatus;
    static kSecACLAuthorizationDecrypt: CFStringRef;
}

/// 항목을 만들 때 신뢰 앱 목록.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TrustList {
    /// 빈 배열. 어떤 앱도 확인 없이 읽지 못한다.
    Empty,
    /// null. 생성한 앱이 신뢰 앱이 된다. 강화가 아니며 시험의 대조군으로만 쓴다.
    #[cfg(test)]
    CreatorOnly,
}

/// 읽기 권한(decrypt) ACL에 기록된 신뢰 앱 목록.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TrustedApps {
    /// 목록이 null이다. 모든 앱이 확인 없이 읽는다.
    Any,
    /// 목록이 비었다.
    Nobody,
    Listed(usize),
}

/// 다 쓰면 `CFRelease`하는 소유 참조.
struct Owned(CFTypeRef);

impl Owned {
    fn new(raw: CFTypeRef) -> Option<Self> {
        (!raw.is_null()).then_some(Self(raw))
    }
}

impl Owned {
    fn as_mut<T>(&self) -> *mut T {
        self.0.cast_mut().cast()
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: `new`가 null이 아닌 소유 참조만 담는다.
        unsafe { CFRelease(self.0) };
    }
}

/// 확인 창을 띄우지 못하게 하고, 끝나면 원래 값으로 돌려놓는다. 이 값은 프로세스 전체에 걸린다.
struct SilentGuard(u8);

impl SilentGuard {
    fn enter() -> Self {
        let mut previous = 1u8;
        // SAFETY: 출력 인자는 유효한 지역 변수다.
        unsafe {
            SecKeychainGetUserInteractionAllowed(&mut previous);
            SecKeychainSetUserInteractionAllowed(0);
        }
        Self(previous)
    }
}

impl Drop for SilentGuard {
    fn drop(&mut self) {
        // SAFETY: 값 하나를 넘기는 호출이다.
        unsafe { SecKeychainSetUserInteractionAllowed(self.0) };
    }
}

fn classify(status: OsStatus) -> ItemError {
    match status {
        ERR_SEC_ITEM_NOT_FOUND => ItemError::NotFound,
        ERR_SEC_DUPLICATE_ITEM => ItemError::Duplicate,
        ERR_SEC_USER_CANCELED => ItemError::Cancelled,
        ERR_SEC_AUTH_FAILED => ItemError::Denied,
        ERR_SEC_INTERACTION_NOT_ALLOWED => ItemError::InteractionRequired,
        _ => ItemError::Failed,
    }
}

fn check(status: OsStatus) -> Result<(), ItemError> {
    if status == 0 {
        Ok(())
    } else {
        Err(classify(status))
    }
}

/// 서비스와 계정으로 고르는 항목 하나.
#[derive(Debug)]
pub(super) struct MacKeychainItem {
    service: String,
    account: String,
}

impl MacKeychainItem {
    pub(super) fn new(service: &str, account: &str) -> Self {
        Self {
            service: service.to_owned(),
            account: account.to_owned(),
        }
    }

    /// 지정한 신뢰 앱 목록으로 항목을 만든다. 같은 항목이 있으면 `Duplicate`.
    pub(super) fn create_with(&self, value: &str, trust: TrustList) -> Result<(), ItemError> {
        let descriptor = Owned::new(cf_string(&self.service).cast()).ok_or(ItemError::Failed)?;
        let empty = match trust {
            TrustList::Empty => Some(empty_array()?),
            #[cfg(test)]
            TrustList::CreatorOnly => None,
        };
        let trusted: CFArrayRef = empty.as_ref().map_or(ptr::null(), |array| array.0.cast());
        let mut access: SecAccessRef = ptr::null_mut();
        // SAFETY: 인자는 모두 살아 있는 참조이고 `access`는 유효한 출력 자리다.
        check(unsafe { SecAccessCreate(descriptor.0.cast(), trusted, &mut access) })?;
        let access = Owned::new(access.cast()).ok_or(ItemError::Failed)?;

        let mut attrs = [
            attribute(SERVICE_ATTR, self.service.as_bytes()),
            attribute(ACCOUNT_ATTR, self.account.as_bytes()),
        ];
        let list = SecKeychainAttributeList {
            count: 2,
            attr: attrs.as_mut_ptr(),
        };
        let length = u32::try_from(value.len()).map_err(|_| ItemError::Failed)?;
        let mut item: SecKeychainItemRef = ptr::null_mut();
        // SAFETY: `attrs`와 `value`는 호출 동안 살아 있고, 기본 키체인은 null로 고른다.
        let status = unsafe {
            SecKeychainItemCreateFromContent(
                GENERIC_PASSWORD_CLASS,
                &list,
                length,
                value.as_ptr().cast(),
                ptr::null_mut(),
                access.as_mut(),
                &mut item,
            )
        };
        check(status)?;
        drop(Owned::new(item.cast()));
        Ok(())
    }

    /// 항목의 읽기 권한 ACL에 기록된 신뢰 앱 목록. 비밀 값을 읽지 않으므로 확인 창이 없다.
    pub(super) fn trusted_apps(&self) -> Result<TrustedApps, ItemError> {
        let item = self.find(false)?.1.ok_or(ItemError::Failed)?;
        let mut access: SecAccessRef = ptr::null_mut();
        // SAFETY: `item`은 살아 있는 항목 참조다.
        check(unsafe { SecKeychainItemCopyAccess(item.as_mut(), &mut access) })?;
        let access = Owned::new(access.cast()).ok_or(ItemError::Failed)?;
        // SAFETY: 상수 태그와 살아 있는 access를 넘긴다.
        let acls = unsafe {
            SecAccessCopyMatchingACLList(access.as_mut(), kSecACLAuthorizationDecrypt.cast())
        };
        let acls = Owned::new(acls.cast()).ok_or(ItemError::Failed)?;
        // SAFETY: 살아 있는 배열이다.
        let count = unsafe { CFArrayGetCount(acls.0.cast()) };
        let mut nobody = false;
        for index in 0..count {
            let mut apps: CFArrayRef = ptr::null();
            let mut description: CFStringRef = ptr::null();
            let mut selector = 0u16;
            // SAFETY: 인덱스는 범위 안이고 출력 인자는 유효한 지역 변수다.
            let status = unsafe {
                SecACLCopyContents(
                    CFArrayGetValueAtIndex(acls.0.cast(), index).cast_mut(),
                    &mut apps,
                    &mut description,
                    &mut selector,
                )
            };
            check(status)?;
            drop(Owned::new(description.cast()));
            let Some(apps) = Owned::new(apps.cast()) else {
                return Ok(TrustedApps::Any);
            };
            // SAFETY: 살아 있는 배열이다.
            let listed = unsafe { CFArrayGetCount(apps.0.cast()) };
            if listed > 0 {
                return Ok(TrustedApps::Listed(
                    usize::try_from(listed).unwrap_or(usize::MAX),
                ));
            }
            nobody = true;
        }
        Ok(if nobody {
            TrustedApps::Nobody
        } else {
            TrustedApps::Any
        })
    }

    /// 확인 창을 막은 채 읽는다. 확인이 필요하면 `InteractionRequired`로 끝나며 값은 돌려주지 않는다.
    pub(super) fn read_silently(&self) -> Result<String, ItemError> {
        let _guard = SilentGuard::enter();
        // 창을 막으면 확인이 필요한 읽기는 거부(errSecAuthFailed)로 끝난다. 사용자의 거부와 구분되지 않으므로 같은 뜻으로 묶는다.
        match self.read_secret() {
            Err(ItemError::Denied) => Err(ItemError::InteractionRequired),
            other => other,
        }
    }

    fn read_secret(&self) -> Result<String, ItemError> {
        let (data, _) = self.find(true)?;
        data.ok_or(ItemError::Failed)
    }

    /// 항목 참조(와 요청하면 값)를 찾는다. 값을 요청할 때만 읽기 권한을 확인한다.
    fn find(&self, with_secret: bool) -> Result<(Option<String>, Option<Owned>), ItemError> {
        let service = self.service.as_bytes();
        let account = self.account.as_bytes();
        let mut length = 0u32;
        let mut data: *mut c_void = ptr::null_mut();
        let mut item: SecKeychainItemRef = ptr::null_mut();
        let (length_out, data_out) = if with_secret {
            (&raw mut length, &raw mut data)
        } else {
            (ptr::null_mut(), ptr::null_mut())
        };
        // SAFETY: 길이와 포인터는 호출 동안 유효하고 출력 인자는 null이거나 유효한 지역 변수다.
        let status = unsafe {
            SecKeychainFindGenericPassword(
                ptr::null(),
                u32::try_from(service.len()).map_err(|_| ItemError::Failed)?,
                service.as_ptr().cast(),
                u32::try_from(account.len()).map_err(|_| ItemError::Failed)?,
                account.as_ptr().cast(),
                length_out,
                data_out,
                &mut item,
            )
        };
        check(status)?;
        let item = Owned::new(item.cast());
        if !with_secret {
            return Ok((None, item));
        }
        // SAFETY: 성공했으면 `data`가 `length` 바이트를 가리킨다. 복사한 뒤 바로 해제한다.
        let bytes =
            unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length as usize) }.to_vec();
        // SAFETY: 위 호출이 돌려준 버퍼다.
        unsafe { SecKeychainItemFreeContent(ptr::null_mut(), data) };
        let text = String::from_utf8(bytes).map_err(|_| ItemError::Failed)?;
        Ok((Some(text), item))
    }
}

impl HardenedKeychain for MacKeychainItem {
    fn create(&self, value: &str) -> Result<(), ItemError> {
        self.create_with(value, TrustList::Empty)
    }

    fn read(&self) -> Result<String, ItemError> {
        self.read_secret()
    }

    fn delete(&self) -> Result<(), ItemError> {
        match self.find(false) {
            Ok((_, Some(item))) => {
                // SAFETY: 살아 있는 항목 참조다.
                check(unsafe { SecKeychainItemDelete(item.as_mut()) })
            }
            Ok(_) => Err(ItemError::Failed),
            Err(ItemError::NotFound) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn verify_policy(&self) -> Result<(), ItemError> {
        if self.trusted_apps()? != TrustedApps::Nobody {
            return Err(ItemError::PolicyMismatch);
        }
        match self.read_silently() {
            Err(ItemError::InteractionRequired) => Ok(()),
            Ok(_) => Err(ItemError::PolicyMismatch),
            Err(error) => Err(error),
        }
    }
}

fn cf_string(text: &str) -> CFStringRef {
    let Ok(c_text) = std::ffi::CString::new(text) else {
        return ptr::null();
    };
    // SAFETY: NUL로 끝나는 UTF-8 문자열을 복사한다.
    unsafe { CFStringCreateWithCString(ptr::null(), c_text.as_ptr(), kCFStringEncodingUTF8) }
}

fn empty_array() -> Result<Owned, ItemError> {
    // SAFETY: 원소 0개 배열이라 값 포인터는 쓰이지 않는다.
    let array = unsafe {
        CFArrayCreate(
            ptr::null(),
            ptr::null_mut(),
            0,
            &raw const kCFTypeArrayCallBacks,
        )
    };
    Owned::new(array.cast()).ok_or(ItemError::Failed)
}

fn attribute(tag: u32, bytes: &[u8]) -> SecKeychainAttribute {
    SecKeychainAttribute {
        tag,
        length: u32::try_from(bytes.len()).unwrap_or(u32::MAX),
        data: bytes.as_ptr().cast_mut().cast(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 시험용 가짜 항목 이름. 실제 항목과 겹치지 않는다.
    fn fake_name() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        format!("saturn-defense-test-{}-{nanos}", std::process::id())
    }

    /// 끝나면 가짜 항목을 지운다.
    struct Fake(MacKeychainItem);

    impl Fake {
        fn new() -> Self {
            let name = fake_name();
            Self(MacKeychainItem::new(&name, &name))
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = self.0.delete();
        }
    }

    /// 사람 클릭 없이 판정하는 실측. 확인 창이 필요한 읽기는 창 대신 `InteractionRequired`로 끝난다.
    #[test]
    #[ignore = "login 키체인에 가짜 항목을 만든다: cargo test -p saturn-engine keychain -- --ignored"]
    fn empty_list_blocks_silent_read_but_null_list_does_not() {
        let hardened = Fake::new();
        hardened
            .0
            .create_with("fake-secret", TrustList::Empty)
            .unwrap();
        assert_eq!(hardened.0.trusted_apps().unwrap(), TrustedApps::Nobody);
        let silent = hardened.0.read_silently();
        assert!(
            matches!(silent, Err(ItemError::InteractionRequired)),
            "{silent:?}"
        );
        assert!(hardened.0.verify_policy().is_ok());

        let control = Fake::new();
        control
            .0
            .create_with("fake-secret", TrustList::CreatorOnly)
            .unwrap();
        assert_eq!(control.0.read_silently().unwrap(), "fake-secret");
        assert!(matches!(
            control.0.verify_policy(),
            Err(ItemError::PolicyMismatch)
        ));

        for fake in [&hardened, &control] {
            fake.0.delete().unwrap();
            assert!(matches!(fake.0.read_silently(), Err(ItemError::NotFound)));
        }
    }

    #[test]
    fn status_codes_map_to_item_errors() {
        assert!(matches!(
            classify(ERR_SEC_USER_CANCELED),
            ItemError::Cancelled
        ));
        assert!(matches!(classify(ERR_SEC_AUTH_FAILED), ItemError::Denied));
        assert!(matches!(
            classify(ERR_SEC_INTERACTION_NOT_ALLOWED),
            ItemError::InteractionRequired
        ));
        assert!(matches!(
            classify(ERR_SEC_ITEM_NOT_FOUND),
            ItemError::NotFound
        ));
        assert!(matches!(classify(-1), ItemError::Failed));
    }
}
