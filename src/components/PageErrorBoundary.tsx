import { Component, type ReactNode } from "react";

export default class PageErrorBoundary extends Component<
  { children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    if (this.state.failed) {
      return (
        <div role="alert" style={{ padding: 24 }}>
          <h2>当前页面遇到问题</h2>
          <p>可以重新打开此页，或通过左侧菜单切换页面。未保存的编辑需要重新填写。</p>
          <button onClick={() => this.setState({ failed: false })}>重新打开此页</button>
        </div>
      );
    }
    return this.props.children;
  }
}
