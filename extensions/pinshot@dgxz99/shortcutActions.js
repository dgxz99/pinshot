/** 扩展首选项、GSettings 与全局快捷键注册共用的操作清单 */
export const SHORTCUT_ACTIONS = [
    {key: 'pinshot-capture-pin', title: '区域截图并贴图', command: 'capture-pin'},
    {key: 'pinshot-pin-clipboard', title: '剪贴板贴图', command: 'pin-clipboard',
        description: '先在原应用复制文字或图片，再按此快捷键'},
    {key: 'pinshot-capture-edit', title: '区域截图后标注', command: 'capture-edit'},
    {key: 'pinshot-longshot', title: '滚动长截图', command: 'longshot',
        description: '框选正文，滚动后点「完成」'},
    {key: 'pinshot-longshot-choose-screen', title: '滚动长截图：重新选择屏幕', command: 'longshot',
        args: ['--choose-screen'], description: '不使用上次记住的屏幕，打开 GNOME 共享选择器'},
    {key: 'pinshot-toggle-hide', title: '显示／隐藏全部贴图'},
    {key: 'pinshot-restore-last', title: '恢复最近一张贴图', command: 'restore-last'},
    {key: 'pinshot-unghost-all', title: '解除全部点击穿透',
        description: '不需要点击贴图，穿透期间也能使用'},
];
