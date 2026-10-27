import {AbsoluteFill, Img, interpolate, Sequence, staticFile, useCurrentFrame} from "remotion";

const FPS = 30;
const s = (sec: number) => Math.round(sec * FPS);
const FADE = 8;

// Stills are real captures of the app (see demo/README.md); captions are the story.
const SCENES = [
  {img: "scan.png", sec: 3, title: "1 · Scan", text: "Read-only. Nothing is touched while it looks."},
  {img: "results.png", sec: 4, title: "2 · Scored, with reasons", text: "Every suggestion says why it is safe — or asks you."},
  {img: "confirm.png", sec: 3.5, title: "3 · Quarantine, not delete", text: "Files move to quarantine first. Nothing is deleted."},
  {img: "restore.png", sec: 4, title: "4 · One-click restore", text: "Changed your mind? Put everything back."},
] as const;

const INTRO = 3;
const PROMISE = 3.5;
export const DURATION = s(INTRO + PROMISE + SCENES.reduce((a, c) => a + c.sec, 0));

const BG = "#0d0e11";
const FG = "#eceef2";
const MUTED = "#9aa3b2";
const ACCENT = "#19c3d6";
const font = "Segoe UI, system-ui, sans-serif";

// Fade in/out over a sequence's own length.
const useFade = (len: number) => {
  const f = useCurrentFrame();
  return interpolate(f, [0, FADE, len - FADE, len], [0, 1, 1, 0], {extrapolateLeft: "clamp", extrapolateRight: "clamp"});
};

const Intro = ({len}: {len: number}) => {
  const f = useCurrentFrame();
  const o = useFade(len);
  const scale = interpolate(f, [0, len], [0.92, 1]);
  return (
    <AbsoluteFill style={{alignItems: "center", justifyContent: "center", opacity: o, fontFamily: font}}>
      <Img src={staticFile("logo.png")} style={{width: 220, height: 220, transform: `scale(${scale})`}} />
      <div style={{color: FG, fontSize: 64, fontWeight: 700, marginTop: 24}}>WinKnife</div>
      <div style={{color: MUTED, fontSize: 26, marginTop: 8}}>The Windows Swiss-army knife that asks first</div>
    </AbsoluteFill>
  );
};

const Promise = ({len}: {len: number}) => {
  const f = useCurrentFrame();
  const o = useFade(len);
  const lines = ["Analyze first.", "Never delete without consent.", "Everything reversible."];
  return (
    <AbsoluteFill style={{alignItems: "center", justifyContent: "center", opacity: o, fontFamily: font}}>
      {lines.map((t, i) => {
        const a = interpolate(f, [i * 18, i * 18 + 14], [0, 1], {extrapolateLeft: "clamp", extrapolateRight: "clamp"});
        return (
          <div key={t} style={{color: i === 2 ? ACCENT : FG, fontSize: 56, fontWeight: 700, margin: "10px 0", opacity: a, transform: `translateY(${(1 - a) * 16}px)`}}>
            {t}
          </div>
        );
      })}
    </AbsoluteFill>
  );
};

const Scene = ({scene, len}: {scene: (typeof SCENES)[number]; len: number}) => {
  const f = useCurrentFrame();
  const o = useFade(len);
  
  return (
    <AbsoluteFill style={{opacity: o, fontFamily: font, alignItems: "center"}}>
      <div style={{marginTop: 20, width: 1000, height: 625, borderRadius: 12, overflow: "hidden", border: "1px solid #2a2d35", boxShadow: "0 10px 40px #0008"}}>
        <Img src={staticFile(scene.img)} style={{width: 1000, height: 625}} />
      </div>
      <div style={{marginTop: 14, textAlign: "center"}}>
        <span style={{color: ACCENT, fontSize: 28, fontWeight: 700}}>{scene.title}</span>
        <span style={{color: MUTED, fontSize: 24}}>{"  —  "}{scene.text}</span>
      </div>
    </AbsoluteFill>
  );
};

export const Demo = () => {
  let at = 0;
  const seq = (sec: number, node: (len: number) => React.ReactNode) => {
    const len = s(sec);
    const el = (
      <Sequence key={at} from={at} durationInFrames={len}>
        {node(len)}
      </Sequence>
    );
    at += len;
    return el;
  };
  return (
    <AbsoluteFill style={{backgroundColor: BG}}>
      {seq(INTRO, (l) => <Intro len={l} />)}
      {seq(PROMISE, (l) => <Promise len={l} />)}
      {SCENES.map((sc) => seq(sc.sec, (l) => <Scene scene={sc} len={l} />))}
    </AbsoluteFill>
  );
};
