const HtmlWebpackPlugin = require('html-webpack-plugin');
const MiniCssExtractPlugin = require('mini-css-extract-plugin');
const WasmPackPlugin = require('@wasm-tool/wasm-pack-plugin');
const path = require('path');
const fs = require('fs');

const interval = setInterval(() => console.log('keepalive'), 1000 * 60 * 5);

module.exports = (env = {}) => {
    const config = {
        entry: './src/index.js',
        output: {
            path: path.join(__dirname, 'dist'),
            filename: 'index.js'
        },
        mode: 'development',
        resolve: {
            alias: {
                rustpython: path.resolve(
                    __dirname,
                    env.rustpythonPkg || '../lib/pkg'
                )
            }
        },
        module: {
            rules: [
                {
                    test: /\.css$/,
                    use: [MiniCssExtractPlugin.loader, 'css-loader']
                }
            ]
        },
        plugins: [
            new HtmlWebpackPlugin({
                filename: 'index.html',
                template: 'src/index.ejs',
                templateParameters: {
                    snippets: fs
                        .readdirSync(path.join(__dirname, 'snippets'))
                        .map(filename =>
                            path.basename(filename, path.extname(filename))
                        ),
                    defaultSnippetName: 'fibonacci',
                    defaultSnippet: fs.readFileSync(
                        path.join(__dirname, 'snippets/fibonacci.py')
                    )
                }
            }),
            new MiniCssExtractPlugin({
                filename: 'styles.css'
            }),
            {
                apply(compiler) {
                    compiler.hooks.done.tap('clearInterval', () => {
                        clearInterval(interval);
                    });
                }
            }
        ]
    };
    if (!env.noWasmPack) {
        config.plugins.push(
            new WasmPackPlugin({
                crateDirectory: path.join(__dirname, '../lib')
            })
        );
    }
    return config;
};
