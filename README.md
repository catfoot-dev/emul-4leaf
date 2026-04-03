# emul-4leaf

**4Leaf Browser 에뮬레이터 프로젝트. Rust를 활용해서 크로스 플랫폼 에뮬레이션을 개발합니다.**

> 해당 애플리케이션을 실행하기 위해서는 4leaf Browser 클라이언트 파일은 별도로 준비하셔야 합니다.

---

## 하이라이트

- [x] **크로스 플랫폼**: Windows / macOS / Linux
- [x] **서버 내장**: 외부 연결 없이 실행할 수 있습니다.
- [x] **오픈 소스**: 오픈소스에 기여해주세요.

> 상태: **작업 중 / 실험 상태** — `v1.0` 버전까지 구조가 변경될 수 있습니다.

---

## 기능

- 디버깅 윈도우 (레지스터, 메모리 스택 표시)
- 내장 서버 패킷 캡쳐
- 크로스 플랫폼 지원 *(예정)*

---

## 시작하기

### Install (소스 코드로)

#### 1) 사전 준비

- Rust (stable): http://rustup.rs
- 4Leaf Browser: 아카이브나 커뮤니티를 통해 준비해주세요. 소스 코드를 받으신 후 4Leaf Browser 클라이언트 파일 전체를 소스 코드가 있는 Resources 폴더 안에 복사합니다.

#### 2) 빌드 & 실행

```bash
git clone https://github.com/catfoot-dev/emul-4leaf
cd emul-4leaf

# 4Leaf Brower Copy
# windows
mkdir Resources
xcopy "C:\\Program Files\\4Leaf" ".\\Resources" /e /h /k /y
# macos / linux
# ~/4Leaf 경로에 클라이언트 파일이 존재할 경우
cp -rf ~/4Leaf ./Resources

# Debug
cargo run

# Release
cargo run --release
```

#### 3) 실행 파일 빌드

```bash
cargo build --release
```

---

## 라이선스

이 프로젝트는 MIT 라이선스에 따라 라이선스가 부여됩니다.
자세한 것은 [LICENSE](./LICENSE)를 참고바랍니다.
