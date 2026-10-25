import { useCallback, useEffect, useState } from "react";
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

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const conn = await api.getConnection();
      if (conn) {
        setBaseUrl(conn.baseUrl);
        setEmail(conn.email);
        setLoaded(true);
      } else {
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
    if (!baseUrl.trim() || !email.trim() || !token.trim()) {
      setError("테스트 전에 Base URL, 이메일, API 토큰을 모두 입력하세요. (입력값을 저장 후 테스트합니다)");
      return;
    }
    setTesting(true);
    setError(null);
    setNotice(null);
    try {
      // 항상 화면의 입력값으로 저장한 뒤 테스트한다 (stale keychain 자격증명 방지).
      await api.saveConnection({ baseUrl: baseUrl.trim(), email: email.trim(), token: token.trim() });
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
        에서 만들 수 있으며, 시스템 키체인(macOS Keychain)에만 저장됩니다.
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
