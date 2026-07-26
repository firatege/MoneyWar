import { Logo } from "../brand/Logo";
import "./footer.css";

interface Props {
  onHelp: () => void;
}

export function Footer({ onHelp }: Props) {
  return (
    <footer className="ftr">
      <div className="ftr__left">
        <Logo size={15} className="ftr__logo" />
        <span className="ftr__brand">MoneyWar</span>
        <span className="ftr__tag">canlı ekonomi simülasyonu</span>
      </div>

      <nav className="ftr__nav" aria-label="bağlantılar">
        <button className="ftr__link ftr__link--btn" onClick={onHelp}>
          nasıl çalışır
        </button>
        <span className="ftr__sep">·</span>
        <a
          className="ftr__link"
          href="https://byfeb.com"
          target="_blank"
          rel="noreferrer noopener"
        >
          byfeb.com
        </a>
        <span className="ftr__sep">·</span>
        <a
          className="ftr__link"
          href="https://github.com/firatege/MoneyWar"
          target="_blank"
          rel="noreferrer noopener"
        >
          kaynak
        </a>
        <span className="ftr__sep">·</span>
        <a
          className="ftr__link"
          href="/api/log?tail=4000"
          target="_blank"
          rel="noreferrer noopener"
        >
          log
        </a>
        <span className="ftr__sep">·</span>
        <span className="ftr__copy">© 2026 Fırat Ege</span>
      </nav>
    </footer>
  );
}
