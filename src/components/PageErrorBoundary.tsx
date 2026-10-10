import { Component, type ReactNode } from "react";

export default class PageErrorBoundary extends Component<
  { children: ReactNode },
  { failed: boolean; reloadRequired: boolean }
> {
  state = { failed: false, reloadRequired: false };

  static getDerivedStateFromError(error: unknown) {
    const message = error instanceof Error ? error.message : String(error);
    // React.lazy 和浏览器模块表会记住加载失败，仅重新渲染无法重新请求文件。
    const reloadRequired = /Failed to fetch dynamically imported module|error loading dynamically imported module|Importing a module script failed|Failed to load module script|Unable to preload CSS/i.test(message);
    return { failed: true, reloadRequired };
  }

  render() {
    if (this.state.failed) {
      return (
        <div role="alert" style={{ padding: 24 }}>
          <h2>当前页面遇到问题</h2>
          <p>{this.state.reloadRequired ? "页面文件未能加载，请重新加载界面。未保存的编辑需要重新填写。" : "可以重新打开此页，或通过左侧菜单切换页面。未保存的编辑需要重新填写。"}</p>
          <button onClick={() => {
            if (this.state.reloadRequired) window.location.reload();
            else this.setState({ failed: false, reloadRequired: false });
          }}>{this.state.reloadRequired ? "重新加载界面" : "重新打开此页"}</button>
        </div>
      );
    }
    return this.props.children;
  }
}
