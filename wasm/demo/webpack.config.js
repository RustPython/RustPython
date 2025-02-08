const HtmlWebpackPlugin = require('html-webpack-plugin');
const MiniCssExtractPlugin = require('mini-css-extract-plugin');
const WasmPackPlugin = require('@wasm-tool/wasm-pack-plugin');

const path = require('path');
const fs = require('fs');

module.exports = (env = {}) => {
    const config = {
        entry: './src/index.js',
        output: {
            path: path.join(__dirname, 'dist'),
            filename: 'index.js',
            clean: true,
        },
        mode: 'development',
        resolve: {
            alias: {
                rustpython: path.resolve(
                    __dirname,
                    env.rustpythonPkg || '../lib/pkg',
                ),
            },
        },
        module: {
            rules: [
                {
                    test: /\.css$/,
                    use: [MiniCssExtractPlugin.loader, 'css-loader'],
                },
                {
                    resourceQuery: '?raw',
                    type: 'asset/source',
                },
                {
                    test: /\.wasm$/,
                    type: 'webassembly/async',
                },
            ],
        },
        plugins: [
            new HtmlWebpackPlugin({
                filename: 'index.html',
                template: 'src/index.ejs',
                templateParameters: {
                    snippets: fs
                        .readdirSync(path.join(__dirname, 'snippets'))
                        .map((filename) =>
                            path.basename(filename, path.extname(filename)),
                        ),
                    defaultSnippetName: 'fibonacci',
                    defaultSnippet: fs.readFileSync(
                        path.join(__dirname, 'snippets/fibonacci.py'),
                    ),
                },
            }),
            new MiniCssExtractPlugin({
                filename: 'styles.css',
            }),
        ],
        experiments: {
            asyncWebAssembly: true,
        },
    };
    if (!env.noWasmPack) {
        config.plugins.push(
            new WasmPackPlugin({
                crateDirectory: path.join(__dirname, '../lib'),
            }),
        );
    }
    return config;
};
