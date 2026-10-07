# Claude Usage Widget

메뉴바에서 **Claude 사용량이 얼마나 남았는지** 바로 보여주는 작은 앱이에요.
Claude Pro / Max 구독의 **5시간 세션 한도**와 **주간 한도**가 몇 % 남았는지, 언제 초기화되는지 알려줍니다.

```
메뉴바:  ◔ 5h 72% · 7d 39%

클릭하면:
  5시간 세션: 72% 남음 (28% 사용)
      2시간 13분 후 초기화
  주간 (전체 모델): 39% 남음 (61% 사용)
      3일 4시간 후 초기화
  주간 (Sonnet): 88% 남음 (12% 사용)
      3일 4시간 후 초기화
  마지막 확인: 13:05
  ─────────────
  지금 새로고침
  자세히 보기…
  ✓ 로그인 시 자동 실행
  ─────────────
  종료
```

<img src="assets/screenshot-details.png" width="360" alt="자세히 보기 창">

- 지원: **macOS 11+** (Apple Silicon / Intel 공용), Windows (실험적)
- 2분마다 자동으로 갱신돼요.
- 사용량을 *조회*만 하므로 Claude 사용량을 소모하지 않아요.

## 필요한 것

[Claude Code](https://docs.claude.com/en/docs/claude-code)가 설치되어 있고, Claude 구독 계정으로 로그인되어 있어야 해요.
터미널에서 `claude`를 한 번 실행해 로그인하면 됩니다. 이 앱은 그 로그인 정보를 그대로 읽어서 쓰기 때문에 API 키를 따로 넣을 필요가 없어요.

## 설치 (macOS)

1. [Releases](https://github.com/smtodj/claude-usage-widget/releases/latest) 페이지에서 `Claude.Usage_x.y.z_universal.dmg`를 받아요.
2. dmg를 열고 **Claude Usage**를 `응용 프로그램` 폴더로 끌어다 놓아요.
3. 아직 Apple 공증(notarization)을 받지 않은 앱이라 처음 실행할 때 macOS가 막을 수 있어요. 터미널에서 아래 명령을 한 번 실행한 뒤 앱을 여세요.

   ```sh
   xattr -dr com.apple.quarantine "/Applications/Claude Usage.app"
   ```

   또는 Finder에서 앱을 **우클릭 → 열기**를 선택해도 돼요.
4. 메뉴바에 게이지 아이콘과 `5h 72% · 7d 39%` 같은 숫자가 나타나면 성공이에요. Dock에는 아이콘이 생기지 않아요.
5. 로그인할 때마다 자동으로 켜지게 하려면 메뉴에서 **로그인 시 자동 실행**을 체크하세요.

처음 실행할 때 "키체인 접근 허용" 창이 뜨면 **항상 허용**을 눌러 주세요. Claude Code가 키체인에 저장한 로그인 토큰을 읽기 위한 것이에요.

## 설치 (Windows, 실험적)

[Releases](https://github.com/smtodj/claude-usage-widget/releases/latest)에서 `Claude.Usage_x.y.z_x64-setup.exe`를 받아 설치하세요.
Windows는 작업 표시줄 트레이에 숫자를 표시할 수 없어서, 트레이 아이콘에 마우스를 올리거나 클릭하면 남은 양이 보여요.

## 동작 방식

- Claude Code의 `/usage` 명령이 쓰는 엔드포인트 `GET https://api.anthropic.com/api/oauth/usage`를 호출해요.
- 인증 토큰은 다음 순서로 찾아요.
  1. 환경 변수 `CLAUDE_USAGE_TOKEN` (직접 지정하고 싶을 때)
  2. macOS 키체인의 `Claude Code-credentials` 항목
  3. `$CLAUDE_CONFIG_DIR/.credentials.json` 또는 `~/.claude/.credentials.json`
- 토큰은 **읽기만** 하고 갱신하거나 저장하지 않아요. 토큰 갱신은 Claude Code가 담당하기 때문에, 이 앱이 건드리면 Claude Code 로그인이 풀릴 수 있어서예요.
- 토큰은 Anthropic API 외의 어디로도 전송되지 않아요.

> ⚠️ 이 엔드포인트는 Anthropic이 공식 문서로 공개한 API가 아니에요. 언제든 바뀔 수 있고, 바뀌면 앱이 오류를 표시해요. 이 프로젝트는 Anthropic과 관련이 없는 비공식 도구예요.

## 문제 해결

| 메뉴에 보이는 메시지 | 해결 방법 |
| --- | --- |
| 로그인 정보를 찾지 못했어요 | 터미널에서 `claude`를 실행해 로그인하세요. |
| 토큰이 만료됐어요 / 인증에 실패했어요(401) | Claude Code 토큰은 몇 시간마다 만료되고, Claude Code를 쓸 때 자동으로 갱신돼요. 터미널에서 `claude`를 한 번 실행한 뒤 **지금 새로고침**을 누르세요. |
| Keychain: ... | 키체인 접근을 거부한 경우예요. 앱을 다시 실행하고 **항상 허용**을 눌러 주세요. |
| 요청이 너무 많아요(429) | 잠시 기다리면 다음 자동 갱신 때 다시 시도해요. |

## 직접 빌드하기

필요한 것: [Rust](https://rustup.rs) (stable), [Node.js](https://nodejs.org) 20+

```sh
git clone https://github.com/smtodj/claude-usage-widget
cd claude-usage-widget
npm install
npm run dev        # 개발 모드로 실행
npm run build      # 설치 파일 생성 (target/release/bundle/)
```

터미널에서 사용량만 확인해 보고 싶다면:

```sh
cargo run -p usage-core --example usage
```

## 프로젝트 구조

```
crates/usage-core/   토큰 읽기, API 호출, 응답 해석 (플랫폼 공통 Rust 라이브러리)
src-tauri/           메뉴바/트레이 앱 (Tauri 2)
ui/                  "자세히 보기" 창 (HTML/CSS/JS)
.github/workflows/   CI, 태그 푸시 시 릴리스 빌드
```

## 새 버전 배포하기

1. `src-tauri/tauri.conf.json`, `Cargo.toml`, `package.json`의 버전을 올려요.
2. 태그를 푸시하면 GitHub Actions가 macOS(universal)와 Windows 설치 파일을 빌드해 Releases에 올려요.

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

## 앞으로의 계획

- [x] macOS 메뉴바
- [x] Windows 트레이 (실험적)
- [ ] iOS / Android: Tauri 2 모바일 빌드로 `usage-core`를 재사용하고, 홈 화면 위젯은 각 플랫폼 네이티브(WidgetKit / Glance)로 붙일 예정이에요. 모바일에는 Claude Code가 없으므로 토큰을 직접 입력하는 방식이 필요해요.
- [ ] 사용량이 일정 비율 아래로 떨어지면 알림

## 라이선스

[MIT](LICENSE)
