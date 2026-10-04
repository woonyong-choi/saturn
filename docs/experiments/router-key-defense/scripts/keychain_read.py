"""시험 항목 하나를 Security 프레임워크로 읽고 성공 여부만 출력한다. 값은 읽자마자 버리고 출력하지 않는다."""
import ctypes
import sys

PREFIX = "saturn-defense-test-"


def main() -> int:
    service = sys.argv[1]
    if not service.startswith(PREFIX):
        print("refused: not a test item")
        return 64
    lib = ctypes.CDLL("/System/Library/Frameworks/Security.framework/Security")
    length = ctypes.c_uint32(0)
    data = ctypes.c_void_p()
    status = lib.SecKeychainFindGenericPassword(
        None,
        len(service),
        service.encode(),
        len("saturn-defense-test"),
        b"saturn-defense-test",
        ctypes.byref(length),
        ctypes.byref(data),
        None,
    )
    if status == 0:
        lib.SecKeychainItemFreeContent(None, data)
        print("accessed")
        return 0
    print(f"status={status}")
    return 1


sys.exit(main())
