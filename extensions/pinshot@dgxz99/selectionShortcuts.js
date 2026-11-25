/** 普通截图与录制工具条共用输出动作和顺序；仅带设置键的动作可绑定框选快捷键 */
export const OUTPUT_ACTIONS = [
    {id: 'edit', key: 'selection-edit', label: '标注'},
    {id: 'pin', key: 'selection-pin', label: '贴图'},
    {id: 'save', label: '保存'},
    {id: 'cancel', label: '取消'},
    {id: 'copy', key: 'selection-copy', label: '复制'},
];

export const SELECTION_ACTIONS = OUTPUT_ACTIONS.filter(({key}) => key);

/** 解析 GTK 加速键格式中的框选按键；Enter 和 Esc 专用于确认与取消 */
export function parseSelectionShortcut(accelerator) {
    const match = /^((?:<(?:control|ctrl|primary|shift)>)*)([a-z]|F(?:[1-9]|1[0-2]))$/i.exec(accelerator);
    if (!match) {
        return null;
    }
    const [, modifiers, keyName] = match;
    const control = /<(control|ctrl|primary)>/i.test(modifiers);
    const shift = /<shift>/i.test(modifiers);
    const key = keyName.length === 1 ? keyName.toLowerCase() : keyName.toUpperCase();
    const label = [control && 'Ctrl', shift && 'Shift', key.toUpperCase()].filter(Boolean).join('+');
    return {key, control, shift, label};
}
