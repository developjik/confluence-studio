import { useCallback, useEffect, useRef, useState } from "react";
import { LogOut, Save, Zap } from "lucide-react";
import { api } from "../api";
import { ErrorBanner, Loading } from "./common";
import { Modal } from "./Modal";

export function SettingsPanel({ onConnectionChanged }: { onConnectionChanged(): void }) {
  const [baseUrl, setBaseUrl] = useState("");
  const [email, setEmail] = useState("");
  const [token, setToken] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  // Last values persisted in the keychain, so "test" can reuse the stored token
  // instead of forcing the user to paste it again on every check.
  const savedRef = useRef<{ baseUrl: string; email: string } | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const conn = await api.getConnection();
      if (conn) {
        setBaseUrl(conn.baseUrl);
        setEmail(conn.email);
        savedRef.current = { baseUrl: conn.baseUrl, email: conn.email };
        setLoaded(true);
      } else {
        savedRef.current = null;
        setLoaded(false);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function handleSave() {
    if (!baseUrl.trim() || !email.trim() || !token.trim()) {
      setError("Base URL, 이메일, API 토큰을 모두 입력하세요.");
      return;
    }
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      await api.saveConnection({ baseUrl: baseUrl.trim(), email: email.trim(), token: token.trim() });
      savedRef.current = { baseUrl: baseUrl.trim(), email: email.trim() };
      setNotice("연결 정보를 시스템 키체인에 저장했습니다.");
      setLoaded(true);
      onConnectionChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleTest() {
    const hasToken = token.trim().length > 0;
    const saved = savedRef.current;
    const endpointChanged = !saved || saved.baseUrl !== baseUrl.trim() || saved.email !== email.trim();
    if (!baseUrl.trim() || !email.trim() || (!hasToken && !saved)) {
      setError("테스트 전에 Base URL, 이메일, API 토큰을 모두 입력하세요. (입력값을 저장 후 테스트합니다)");
      return;
    }
    if (!hasToken && endpointChanged) {
      setError("Base URL이나 이메일을 바꿨다면 API 토큰도 다시 입력하세요.");
      return;
    }
    setTesting(true);
    setError(null);
    setNotice(null);
    try {
      // 입력값이 있으면 항상 화면의 값으로 저장한 뒤 테스트한다 (stale keychain 자격증명 방지).
      // 토큰을 비워 두면 이미 키체인에 있는 자격증명으로 테스트한다.
      if (hasToken) {
        await api.saveConnection({ baseUrl: baseUrl.trim(), email: email.trim(), token: token.trim() });
        savedRef.current = { baseUrl: baseUrl.trim(), email: email.trim() };
      }
      const r = await api.testConnection();
      let msg = r.siteName ? "연결 성공: " + r.siteName : "연결 성공";
      if (r.serverTime) msg += " · 서버 시각 " + r.serverTime;
      setNotice(msg);
      setLoaded(true);
      onConnectionChanged();
    } catch (e) {
      setError("연결 실패: " + (e instanceof Error ? e.message : String(e)));
    } finally {
      setTesting(false);
    }
  }

  async function handleClear() {
    try {
      await api.clearConnection();
      savedRef.current = null;
      setBaseUrl("");
      setEmail("");
      setToken("");
      setLoaded(false);
      setNotice("저장된 연결 정보를 삭제했습니다.");
      setConfirmClear(false);
      onConnectionChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setConfirmClear(false);
    }
  }

  return (
    <div className="settings">
      <h2>Confluence 연결</h2>
      <p className="settings-hint">
        Confluence Cloud 사이트 URL(예: https://your-site.atlassian.net), 이메일, API 토큰을 입력합니다. 토큰은{" "}
        <a href="https://id.atlassian.com/manage-profile/security/api-tokens" target="_blank" rel="noreferrer">
          Atlassian API 토큰 관리 페이지
        </a>
        에서 만들 수 있으며, OS 키체인(macOS Keychain · Windows 자격 증명 관리자)에만 저장되고 앱 파일에는 남지 않습니다.
      </p>

      {error ? <ErrorBanner message={error} /> : null}
      {notice ? (
        <div className="banner success">
          <span className="banner-message">{notice}</span>
        </div>
      ) : null}

      {loading ? (
        <Loading label="연결 정보 확인 중…" />
      ) : (
        <>
          <div className="field">
            <label className="field-label">Base URL</label>
            <input
              className="input"
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              placeholder="https://your-site.atlassian.net"
            />
          </div>
          <div className="field">
            <label className="field-label">이메일</label>
            <input
              className="input"
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="you@example.com"
            />
          </div>
          <div className="field">
            <label className="field-label">API 토큰</label>
            <input
              className="input"
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder={loaded ? "•••••••• (변경할 때만 입력)" : "API 토큰 입력"}
            />
          </div>
          <div className="settings-actions">
            <button className="btn btn-primary" disabled={saving} onClick={() => void handleSave()}>
              <Save size={13} /> {saving ? "저장 중…" : "저장"}
            </button>
            <button className="btn" disabled={testing} onClick={() => void handleTest()}>
              <Zap size={13} /> {testing ? "테스트 중…" : "연결 테스트"}
            </button>
            <button className="btn btn-ghost" disabled={!loaded} onClick={() => setConfirmClear(true)}>
              <LogOut size={13} /> 연결 해제
            </button>
          </div>
        </>
      )}

      {confirmClear ? (
        <Modal
          title="연결 해제"
          danger
          onClose={() => setConfirmClear(false)}
          footer={
            <>
              <button className="btn" onClick={() => setConfirmClear(false)}>
                취소
              </button>
              <button className="btn btn-danger" onClick={() => void handleClear()}>
                삭제
              </button>
            </>
          }
        >
          <p>저장된 연결 정보(Base URL, 이메일, API 토큰)를 키체인에서 삭제할까요?</p>
        </Modal>
      ) : null}
    </div>
  );
}
